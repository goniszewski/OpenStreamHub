import { describe, expect, it } from 'vitest';
import { DemoRuntime } from './demo-runtime';

function setup(options: { failStart?: boolean; neverReady?: boolean } = {}) {
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const ignored: [string, boolean][] = [];
  const keys: number[] = [];
  const ends: unknown[] = [];
  const timeouts: (() => void)[] = [];
  const runtime = new DemoRuntime({
    invoke: (async (command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args });
      if (command === 'demo_start') {
        if (options.failStart) throw new Error('the module is busy');
        if (!options.neverReady) queueMicrotask(() => handlers['service://record']({ payload: { service_id: 'service:ab', record: { type: 'service_start' } } }));
      }
      return undefined;
    }) as never,
    listen: async (event, handler) => { handlers[event] = handler; return () => { delete handlers[event]; }; },
    ignore: (id, on) => { ignored.push([id, on]); },
    render: tile => typeof tile === 'string' ? tile : null,
    setTimeout: callback => { timeouts.push(callback); return timeouts.length; },
    clearTimeout: () => {},
  });
  const commands = () => calls.map(call => call.command);
  const press = (control: number, gesture = 'pressed', id = 'service:ab') => handlers['service://record']?.({ payload: { service_id: id, record: { type: 'event', event: { control, gesture } } } });
  return { runtime, handlers, calls, commands, ignored, keys, ends, timeouts, press, begin: () => runtime.start('AB', key => keys.push(key), end => ends.push(end)) };
}

describe('DemoRuntime', () => {
  it('holds the module, passes key presses on and hides the demo process from the Run panel', async () => {
    const { runtime, commands, ignored, keys, press, begin } = setup();
    await begin();
    expect(runtime.getPhase()).toBe('running');
    expect(commands()).toEqual(['demo_start']);
    expect(ignored).toEqual([['service:ab', true]]);
    press(4); press(5, 'released'); press(6, 'pressed', 'service:other');
    expect(keys).toEqual([4]);
  });

  it('draws only the tiles that changed since they were last drawn', async () => {
    const { runtime, calls, begin } = setup();
    await begin();
    await runtime.render(['a', 'b', 'c']);
    await runtime.render(['a', 'B', 'c']);
    const drawn = calls.filter(call => call.command === 'demo_key_image').map(call => `${call.args!.key}:${call.args!.imageBase64}`);
    expect(drawn).toEqual(['0:a', '1:b', '2:c', '1:B']);
    expect(calls.at(-1)!.args).toMatchObject({ serviceId: 'service:ab' });
  });

  it('hands the module back when it ends, and tells the demo why', async () => {
    const { runtime, commands, ignored, ends, handlers, begin } = setup();
    await begin();
    await runtime.end();
    expect(commands()).toEqual(['demo_start', 'demo_end']);
    expect(ignored.at(-1)).toEqual(['service:ab', false]);
    expect(ends).toEqual([{ reason: 'user' }]);
    expect(runtime.getPhase()).toBe('idle');
    expect(handlers['service://record']).toBeUndefined();
    await runtime.end();
    expect(commands()).toHaveLength(2);
  });

  it('ends itself when the module goes away', async () => {
    const { runtime, commands, ends, handlers, begin } = setup();
    await begin();
    handlers['service://status']({ payload: { service_id: 'service:ab', state: 'exited' } });
    await Promise.resolve(); await Promise.resolve();
    expect(commands()).toEqual(['demo_start', 'demo_end']);
    expect(ends).toEqual([{ reason: 'module-stopped', error: 'The module stopped answering.' }]);
    expect(runtime.getPhase()).toBe('idle');
  });

  it('gives the module back when the start is refused or it never gets ready', async () => {
    const refused = setup({ failStart: true });
    await expect(refused.begin()).rejects.toThrow('busy');
    expect(refused.commands()).toEqual(['demo_start', 'demo_end']);
    expect(refused.runtime.getPhase()).toBe('idle');
    expect(refused.ends).toEqual([]);

    const slow = setup({ neverReady: true });
    const starting = slow.begin();
    await new Promise(resolve => setTimeout(resolve, 0));
    slow.timeouts[0]();
    await expect(starting).rejects.toThrow('did not get ready');
    expect(slow.commands()).toEqual(['demo_start', 'demo_end']);
    expect(slow.ignored.at(-1)).toEqual(['service:ab', false]);
  });
});
