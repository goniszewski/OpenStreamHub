import type { Invoke } from '../profile-session';
import type { Listen } from '../device-feed';
import { serviceIdFor } from '../service-runner';

// How long a demo waits for its module's run to be ready before it gives up.
export const readyTimeoutMs = 8000;

export type DemoPhase = 'idle' | 'starting' | 'running' | 'ending';
export interface DemoEnd { reason: 'user' | 'module-stopped' | 'failed'; error?: string }

export interface DemoRuntimeOptions {
  invoke: Invoke;
  listen: Listen;
  // Keeps the Run panel from showing the demo's process as a run of its own.
  ignore?(serviceId: string, on: boolean): void;
  // The picture of one tile as a base64 PNG, or null when it cannot be drawn.
  render(tile: unknown): string | null;
  setTimeout?: (callback: () => void, ms: number) => unknown;
  clearTimeout?: (handle: unknown) => void;
}

// One demo at a time on one module. The module's own session lets go of it (the backend holds it for the demo), the demo
// runs as a run with no bindings in Test mode, key presses arrive as the run's event records, and pictures are
// drawn through its control channel. Ending hands the module back to its session.
export class DemoRuntime {
  private phase: DemoPhase = 'idle';
  private pathHex: string | null = null;
  private serviceId: string | null = null;
  private unlisten: (() => void)[] = [];
  private drawn = new Map<number, string>();
  private chain: Promise<unknown> = Promise.resolve();
  private onKey: (key: number) => void = () => {};
  private onEnd: (end: DemoEnd) => void = () => {};

  constructor(private readonly options: DemoRuntimeOptions) {}

  getPhase() { return this.phase; }

  // Starts a demo on the module at `pathHex`. Resolves once the module is ready to draw on, or rejects (and leaves
  // everything as it was: the session has its module back).
  async start(pathHex: string, onKey: (key: number) => void, onEnd: (end: DemoEnd) => void): Promise<void> {
    if (this.phase !== 'idle') throw new Error('A demo is already running.');
    this.phase = 'starting';
    this.pathHex = pathHex;
    this.serviceId = serviceIdFor(pathHex);
    this.onKey = onKey;
    this.onEnd = onEnd;
    this.drawn.clear();
    this.options.ignore?.(this.serviceId, true);
    const timers = { set: this.options.setTimeout ?? ((callback, ms) => setTimeout(callback, ms)), clear: this.options.clearTimeout ?? (handle => clearTimeout(handle as ReturnType<typeof setTimeout>)) };
    let ready: () => void = () => {};
    let failed: (error: Error) => void = () => {};
    const readiness = new Promise<void>((resolve, reject) => { ready = resolve; failed = reject; });
    const timeout = timers.set(() => failed(new Error('The module did not get ready for the demo.')), readyTimeoutMs);
    try {
      this.unlisten = await Promise.all([
        this.options.listen('service://record', event => this.onRecord(event.payload, ready)),
        this.options.listen('service://status', event => this.onStatus(event.payload, failed)),
      ]);
      await this.options.invoke<string>('demo_start', { pathHex });
      await readiness;
    } catch (reason) {
      timers.clear(timeout);
      await this.finish(false);
      throw reason instanceof Error ? reason : new Error(String(reason));
    }
    timers.clear(timeout);
    this.phase = 'running';
  }

  // Draws the tiles that differ from what the module shows, one key at a time.
  render(tiles: readonly unknown[]): Promise<void> {
    const drawing = this.chain.then(async () => {
      if (this.phase !== 'running' || !this.serviceId) return;
      for (const [key, tile] of tiles.entries()) {
        const image = this.options.render(tile);
        if (image === null || this.drawn.get(key) === image) continue;
        if (this.phase !== 'running') return;
        await this.options.invoke('demo_key_image', { serviceId: this.serviceId, key, imageBase64: image });
        this.drawn.set(key, image);
      }
    });
    this.chain = drawing.catch(() => {});
    return drawing;
  }

  // Ends the demo and gives the module back.
  async end(reason: DemoEnd['reason'] = 'user', error?: string) {
    if (this.phase === 'idle' || this.phase === 'ending') return;
    const wasRunning = this.phase === 'running';
    await this.finish(true);
    if (wasRunning) this.onEnd({ reason, ...(error ? { error } : {}) });
  }

  private async finish(notify: boolean) {
    this.phase = 'ending';
    this.unlisten.forEach(stop => stop());
    this.unlisten = [];
    const pathHex = this.pathHex;
    try { if (pathHex) await this.options.invoke<void>('demo_end', { pathHex }); }
    finally {
      if (this.serviceId) this.options.ignore?.(this.serviceId, false);
      this.phase = 'idle';
      this.pathHex = null;
      this.serviceId = null;
      if (!notify) this.onEnd = () => {};
    }
  }

  private onRecord(payload: unknown, ready: () => void) {
    const { service_id: id, record } = (payload ?? {}) as { service_id?: unknown; record?: { type?: unknown; event?: { control?: unknown; gesture?: unknown } } };
    if (id !== this.serviceId || !record) return;
    if (record.type === 'service_start') ready();
    else if (record.type === 'event' && record.event?.gesture === 'pressed' && Number.isInteger(record.event.control) && this.phase === 'running') this.onKey(record.event.control as number);
  }

  private onStatus(payload: unknown, failed: (error: Error) => void) {
    const { service_id: id, state, error } = (payload ?? {}) as { service_id?: unknown; state?: unknown; error?: unknown };
    if (id !== this.serviceId || (state !== 'exited' && state !== 'failed')) return;
    const message = typeof error === 'string' ? error : 'The module stopped answering.';
    if (this.phase === 'starting') failed(new Error(message));
    else if (this.phase === 'running') void this.end(state === 'failed' ? 'failed' : 'module-stopped', message);
  }
}
