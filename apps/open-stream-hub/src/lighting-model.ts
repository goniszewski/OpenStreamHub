import type { ServiceModule } from './contracts';
import { formatHexColour, parseHexColour } from './artwork-model';
import { modules } from './ui-model';
import { moduleImage } from './module-controls';
import type { Invoke } from './profile-session';

// The modules the Lighting page can pick: everything the input feed lists,
// plus the base, which takes lighting but no input and has its own listing.
export type LightingModule = ServiceModule | 'm0064';
export interface LightingDevice { path_hex: string; module: LightingModule; serial_number: string | null }


// What lights a module: the dial has a backlight and one colour (feature
// reports); knobs and faders have a colour, an effect and a brightness per
// LED (output reports). The base edge strip and the other modules are not
// lit yet.
export type LightingKind = 'dial' | 'led';

export function lightingKind(module: LightingModule): LightingKind | null {
  if (module === 'm131e') return 'dial';
  return module === 'm003k' || module === 'm005s' || module === 'm0064' ? 'led' : null;
}

// How many LEDs the module has, as the protocol counts them.
export function ledCount(module: LightingModule): number {
  return module === 'm003k' ? 3 : module === 'm005s' ? 5 : module === 'm0064' ? 1 : 0;
}

// The effects the hardware accepted (checked on knobs and faders, 11 October
// 2026). Mode 4 is not in the official app's list but works: a faster strobe.
export const lightingEffects: { value: number; label: string }[] = [
  { value: 1, label: 'Static' },
  { value: 0, label: 'Off' },
  { value: 2, label: 'Breathing' },
  { value: 3, label: 'Strobe, slow' },
  { value: 4, label: 'Strobe, fast' },
  { value: 5, label: 'Cycling' },
  { value: 6, label: 'Random' },
  { value: 7, label: 'Wave' },
  { value: 8, label: 'Water' },
  { value: 9, label: 'Rainbow' },
];
export const defaultEffect = 1;

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
// Whole kit reaches every module of the picked module's kind (every dial, or
// every knob and fader module); with nothing lit-able picked it reaches the dials.
export function lightingTargets(devices: LightingDevice[], scope: LightingScope, selectedPath: string | null): LightingDevice[] {
  const selected = devices.find(device => device.path_hex === selectedPath);
  const selectedKind = selected ? lightingKind(selected.module) : null;
  if (scope === 'module') return selected && selectedKind ? [selected] : [];
  const kind = selectedKind ?? 'dial';
  return devices.filter(device => lightingKind(device.module) === kind);
}

export interface LightingGate {
  desktop: boolean;
  // True when one of the current targets is held by a running profile.
  runOwnedTargets: boolean;
  scope: LightingScope;
  selected: LightingDevice | undefined;
  devices: LightingDevice[];
}

// Why the lighting controls are disabled right now, or null when they are
// usable. The controls always render: an unsupported module shows them
// disabled with the reason instead of hiding them.
export function lightingBlockReason({ desktop, runOwnedTargets, scope, selected, devices }: LightingGate): string | null {
  if (!desktop) return 'The browser preview cannot reach hardware.';
  if (runOwnedTargets) return 'A run is driving a dial. Stop the run before changing its lighting.';
  if (devices.length === 0) return 'No modules are connected. Connect a module to change its lighting.';
  const kind = selected ? lightingKind(selected.module) : 'dial';
  if (scope === 'kit') {
    const wanted = kind ?? 'dial';
    return devices.some(device => lightingKind(device.module) === wanted)
      ? null
      : 'Whole kit lights the encoder dials, or the knob and fader modules, and none is connected.';
  }
  return kind === null && selected
    ? `${moduleShortName(selected.module)} lighting is not available yet. Only the encoder dial, knob and fader modules are lit right now.`
    : null;
}

function moduleShortName(module: LightingModule): string {
  return lightingModuleInfo(module).name;
}

// The name and photo the pickers show for a module.
export function lightingModuleInfo(module: LightingModule): { name: string; image: { src: string; style?: { transform: string } } | null } {
  if (module === 'm0064') return { name: 'MasterHUB base', image: { src: '/modules/base-topdown.svg' } };
  const info = modules.find(item => item.id === module);
  return { name: info?.shortName ?? 'This module', image: info ? moduleImage(info) : null };
}

// The picker's option label: the module name, with a serial tail when the kit
// has several of a kind. Full serials appear where they matter (run targets).
export function lightingDeviceLabel(device: LightingDevice): string {
  const name = lightingModuleInfo(device.module).name;
  const serial = device.serial_number;
  return serial ? `${name} · …${serial.slice(-4)}` : name;
}

// What lighting a module takes, for the picker tiles: a short capability line.
export function lightingCapability(module: LightingModule): string {
  const kind = lightingKind(module);
  if (kind === 'dial') return 'Backlight and colour';
  return kind === 'led' ? `Colour, effect and brightness · ${ledCount(module) === 1 ? '1 LED' : `${ledCount(module)} LEDs`}` : 'Not lit yet';
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

export interface LedLightingRequest { targets: string[]; led: number | null; colour: string; effect: number; brightness: number }

// Knob and fader lighting: the same colour, effect and brightness for one LED
// or every LED of each target module.
export function applyLedLighting(invoke: Invoke, request: LedLightingRequest): Promise<number> {
  return invoke('set_led_lighting', {
    targets: request.targets,
    led: request.led,
    colour: colourChannels(request.colour),
    mode: Math.min(9, Math.max(0, Math.round(request.effect))),
    brightness: clampBacklight(request.brightness),
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

export function appliedLedStatus(count: number): string {
  return count === 1 ? 'Applied to 1 module.' : `Applied to ${count} modules.`;
}

export function resetStatus(count: number): string {
  return count === 1 ? 'The dial returned to its built-in animation.' : `${count} dials returned to their built-in animation.`;
}

export function turnedOffLedStatus(count: number, led: number | null): string {
  const on = count === 1 ? '' : ` on ${count} modules`;
  return led === null ? `The LEDs are off${on}.` : `The LED is off${on}.`;
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
