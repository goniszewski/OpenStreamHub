import type { LiveDevice, ServiceModule } from './contracts';
import { formatHexColour, parseHexColour } from './artwork-model';
import { modules } from './ui-model';
import type { Invoke } from './profile-session';

// Phase 1 lights the encoder dial only: knob rings, base edge and fader
// lighting wait for their protocol (a later phase).
const lightingModule: ServiceModule = 'm131e';

// Where a lighting change lands: the picked module, or every connected dial.
export type LightingScope = 'module' | 'kit';
export const lightingScopeLabels: Record<LightingScope, string> = { module: 'This module', kit: 'Whole kit' };

export const defaultBacklight = 100;
export const defaultColour = '#ffffff';
// The apply is debounced so a dragged slider or colour picker sends one write
// per gesture instead of one per pixel.
export const applyDelayMs = 250;

export const maxColourFavourites = 12;
export const favouritesStorageKey = 'open-stream-hub.lighting-favourites';

// The dials a lighting change reaches, in inventory order. A picked module
// that cannot be lit never becomes a target: its controls are disabled with a
// reason instead (the backend re-checks the module kind on every write).
export function lightingTargets(devices: LiveDevice[], scope: LightingScope, selectedPath: string | null): LiveDevice[] {
  const dials = devices.filter(device => device.module === lightingModule);
  return scope === 'kit' ? dials : dials.filter(device => device.path_hex === selectedPath);
}

export interface LightingGate {
  desktop: boolean;
  // True when one of the current targets is held by a running profile.
  runOwnedTargets: boolean;
  scope: LightingScope;
  selected: LiveDevice | undefined;
  devices: LiveDevice[];
}

// Why the lighting controls are disabled right now, or null when they are
// usable. The controls always render: an unsupported module shows them
// disabled with the reason instead of hiding them.
export function lightingBlockReason({ desktop, runOwnedTargets, scope, selected, devices }: LightingGate): string | null {
  if (!desktop) return 'The browser preview cannot reach hardware.';
  if (runOwnedTargets) return 'A run is driving a dial. Stop the run before changing its lighting.';
  if (devices.length === 0) return 'No modules are connected. Connect a module to change its lighting.';
  if (scope === 'kit') {
    return devices.some(device => device.module === lightingModule)
      ? null
      : 'Whole kit reaches the encoder dial’s lighting only right now, and no dial is connected.';
  }
  return selected && selected.module !== lightingModule
    ? `${moduleShortName(selected.module)} lighting is not available yet. The encoder dial is the only module OpenStreamHub lights right now.`
    : null;
}

function moduleShortName(module: ServiceModule): string {
  return modules.find(item => item.id === module)?.shortName ?? 'This module';
}

// The picker's option label: the module name, with a serial tail when the kit
// has several of a kind. Full serials appear where they matter (run targets).
export function lightingDeviceLabel(device: LiveDevice): string {
  const name = modules.find(item => item.id === device.module)?.shortName ?? device.module;
  const serial = device.serial_number;
  return serial ? `${name} · …${serial.slice(-4)}` : name;
}

// Accepts #rgb, rgb, #rrggbb and rrggbb; a valid colour is stored as #rrggbb
// (the native picker's own format).
export function normaliseHexColour(text: string): string | null {
  const rgb = parseHexColour(text);
  return rgb ? formatHexColour(rgb) : null;
}

// The three raw channel bytes the colour command carries, in the verified
// R, G, B order. Anything unparsable falls back to white.
export function colourChannels(hex: string): [number, number, number] {
  return parseHexColour(hex) ?? [255, 255, 255];
}

export function clampBacklight(value: number): number {
  if (!Number.isFinite(value)) return defaultBacklight;
  return Math.min(100, Math.max(0, Math.round(value)));
}

export interface LightingRequest { targets: string[]; backlight: number; colour: string }

// The one place a lighting apply is shaped, so tests can run it with a fake
// invoke (the page only debounces and calls it).
export function applyLighting(invoke: Invoke, request: LightingRequest): Promise<number> {
  return invoke('set_dial_lighting', {
    targets: request.targets,
    backlight: clampBacklight(request.backlight),
    colour: colourChannels(request.colour),
  }) as Promise<number>;
}

export function resetLighting(invoke: Invoke, targets: string[]): Promise<number> {
  return invoke('reset_dial_lighting', { targets }) as Promise<number>;
}

// Serialises the live applies: writes run one at a time, debounced, and a
// change that lands while a write is running (a dragged slider during a slow
// Whole kit write) marks exactly one trailing apply, so the last control
// values always ship instead of being dropped. `run` reads the caller's
// latest state itself and re-checks its gate, because a trailing apply runs
// after the state that scheduled it.
export function createApplyQueue(run: () => Promise<void>, delayMs: number) {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inFlight = false;
  let pending = false;
  let disposed = false;
  const fire = async (): Promise<void> => {
    if (disposed) return;
    if (inFlight) {
      pending = true;
      return;
    }
    inFlight = true;
    try {
      await run();
    } finally {
      inFlight = false;
      if (pending && !disposed) {
        pending = false;
        await fire();
      }
    }
  };
  return {
    schedule() {
      if (disposed) return;
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => { timer = null; void fire(); }, delayMs);
    },
    fire,
    dispose() {
      disposed = true;
      if (timer !== null) clearTimeout(timer);
    },
  };
}

export function appliedStatus(count: number): string {
  return count === 1 ? 'Applied to 1 dial.' : `Applied to ${count} dials.`;
}

export function resetStatus(count: number): string {
  return count === 1 ? 'The dial returned to its built-in animation.' : `${count} dials returned to their built-in animation.`;
}

export function readColourFavourites(storage: Pick<Storage, 'getItem'> | null): string[] {
  try {
    const parsed: unknown = JSON.parse(storage?.getItem(favouritesStorageKey) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is string => typeof item === 'string')
      .map(item => normaliseHexColour(item))
      .filter((item): item is string => item !== null)
      .slice(0, maxColourFavourites);
  } catch { return []; }
}

export function writeColourFavourites(storage: Pick<Storage, 'setItem'> | null, colours: string[]): void {
  try { storage?.setItem(favouritesStorageKey, JSON.stringify(colours)); } catch { /* favourites stay for this session only */ }
}

// Newest first, deduplicated, capped. An unparsable colour leaves the list alone.
export function addColourFavourite(colours: string[], colour: string): string[] {
  const hex = normaliseHexColour(colour);
  if (!hex) return colours;
  return [hex, ...colours.filter(item => item !== hex)].slice(0, maxColourFavourites);
}

export function removeColourFavourite(colours: string[], colour: string): string[] {
  return colours.filter(item => item !== colour);
}
