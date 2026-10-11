import { clampBacklight, defaultColour, ledCount, lightingKind, normaliseHexColour } from './lighting-model';
import type { LightingDevice } from './lighting-model';
import type { Invoke } from './profile-session';
import { applyLedLighting, applyLighting } from './lighting-model';

// What was last applied to one unit, kept so the rig and the hero image can
// light their art with it and so a module that replugs gets the same writes
// again, the way the official app restores lighting after a replug.
// Keyed by HID path: a replug on the same port keeps the path, and every
// listing (feed, base inventory) speaks in paths.
export interface StoredDial { kind: 'dial'; backlight: number; colour: string }
export interface StoredLed { colour: string; effect: number; brightness: number }
export interface StoredLedModule { kind: 'led'; leds: (StoredLed | null)[] }
export type StoredLighting = StoredDial | StoredLedModule;

// What a module itself reports about its LEDs (brightness and saved effect per LED, never a colour).
export interface LedReadback { brightness: number[]; modes: number[] }
// `records` is what this app wrote; `reported` what modules say about themselves; `shown` is what the art draws:
// the records, with a unit that has none filled in from its report.
export interface LightingSnapshot {
  records: Readonly<Record<string, StoredLighting>>;
  reported: Readonly<Record<string, LedReadback>>;
  shown: Readonly<Record<string, StoredLighting>>;
}

// The colour a unit shows when it reports an effect but no colour: a base red for the colour-shifting effects, which
// move through every hue anyway, and a neutral light for the rest.
export const reportedCyclingColour = '#ff2d55';
export const reportedNeutralColour = '#d8d8e0';

export function lookFromReport(report: LedReadback, count: number): StoredLedModule {
  const leds = Array.from({ length: count }, (_, index) => {
    const effect = report.modes[index];
    const brightness = report.brightness[index];
    if (effect === undefined || brightness === undefined) return null;
    return { colour: effect >= 5 ? reportedCyclingColour : reportedNeutralColour, effect, brightness };
  });
  return { kind: 'led', leds };
}

function snapshotOf(records: Record<string, StoredLighting>, reported: Record<string, LedReadback>): LightingSnapshot {
  const shown = { ...records };
  for (const [path, report] of Object.entries(reported)) {
    if (!(path in shown)) shown[path] = lookFromReport(report, report.modes.length);
  }
  return { records, reported, shown };
}

export const lightingStateStorageKey = 'open-stream-hub.lighting-state';

// The most records kept. A kit has a handful of ports and a replug keeps its path, so this is generous; past it the
// record written longest ago goes first. Write order is the record map's key order (a write moves its record to the
// end), which survives the JSON round trip because a path is a long hex string, never an array index.
export const maxLightingRecords = 64;

// Whether a stored look can ever be applied to a module of this kind: a dial look to a dial, and an LED look to an LED
// module with as many LEDs as the look holds. The backend refuses every other write, so such a record is dead weight.
export function recordFits(record: StoredLighting, module: LightingDevice['module']): boolean {
  if (record.kind === 'dial') return module === 'm131e';
  return lightingKind(module) === 'led' && record.leds.length === ledCount(module);
}

// The newest `limit` records of an ordered map, oldest write dropped first.
function newest(records: Record<string, StoredLighting>, limit = maxLightingRecords): Record<string, StoredLighting> {
  const keys = Object.keys(records);
  if (keys.length <= limit) return records;
  return Object.fromEntries(keys.slice(keys.length - limit).map(key => [key, records[key]]));
}

// The look an apply writes: one dial look, or one LED look per target LED.
export function withDialLook(_existing: StoredLighting | undefined, backlight: number, colour: string): StoredLighting {
  return { kind: 'dial', backlight: clampBacklight(backlight), colour: normaliseHexColour(colour) ?? defaultColour };
}

// `led` null means every LED; `ledCount` sizes a record made fresh. LEDs left
// untouched keep their stored look, or stay unrecorded.
export function withLedLook(existing: StoredLighting | undefined, led: number | null, ledCount: number, look: StoredLed): StoredLighting {
  const previous = existing && existing.kind === 'led' ? existing.leds : [];
  const leds: (StoredLed | null)[] = Array.from({ length: ledCount }, (_, index) => previous[index] ?? null);
  const indexes = led === null ? leds.map((_, index) => index) : [led];
  for (const index of indexes) {
    if (index < 0 || index >= ledCount) continue;
    leds[index] = {
      colour: normaliseHexColour(look.colour) ?? defaultColour,
      effect: Math.min(9, Math.max(0, Math.round(look.effect))),
      brightness: clampBacklight(look.brightness),
    };
  }
  return { kind: 'led', leds };
}

// The first connected unit of the module kind that has a stored look — the rig
// and the hero image draw one tile per kind, so same-kind units share it.
export function kindLighting(records: LightingSnapshot['shown'], devices: LightingDevice[], module: string): StoredLighting | undefined {
  return devices.filter(device => device.module === module).map(device => records[device.path_hex]).find(record => record !== undefined);
}

// The colour a "match modules" shortcut hands to the base's edge strip: the
// first lit LED of the connected knob and fader modules, in inventory order,
// or null when none is lit.
export function matchedModuleColour(records: LightingSnapshot['records'], devices: LightingDevice[]): string | null {
  for (const device of devices) {
    if (device.module !== 'm003k' && device.module !== 'm005s') continue;
    const record = records[device.path_hex];
    if (!record || record.kind !== 'led') continue;
    for (const led of record.leds) {
      if (led && led.effect !== 0 && led.brightness > 0) return led.colour;
    }
  }
  return null;
}

// What the art draws for a LED nobody knows the look of.
export const unknownLed = 'unknown';

// The colour each of the module's LEDs shows in the art (a knob's ring, a fader's glow strip): the LED's colour when
// it is lit — an effect other than Off, at a brightness above zero — null when it is off, and `unknownLed` when
// there is neither a stored look nor a report. The colour-shifting effects move every LED through the same hues together, as the
// hardware does, so the art's hue shift keeps them in step.
export function litLedColours(record: StoredLighting | undefined, count: number): (string | null)[] {
  return Array.from({ length: count }, (_, index) => {
    const led = record && record.kind === 'led' ? record.leds[index] : undefined;
    if (!led) return unknownLed;
    if (led.effect === 0 || led.brightness <= 0) return null;
    return led.colour;
  });
}

// How a module's lit LEDs move, for the art: 'cycle' for the colour-shifting effects, 'blink' for the strobes,
// 'pulse' for breathing, null when nothing lit moves. Static is still, Off is dark.
export type LightMotion = 'cycle' | 'blink' | 'pulse';
export function lightMotion(record: StoredLighting | undefined, count: number): LightMotion | null {
  const effects = Array.from({ length: count }, (_, index) => record && record.kind === 'led' ? record.leds[index] : undefined)
    .filter(led => led && led.effect !== 0 && led.brightness > 0).map(led => led!.effect);
  if (effects.some(effect => effect >= 5)) return 'cycle';
  if (effects.some(effect => effect === 3 || effect === 4)) return 'blink';
  if (effects.includes(2)) return 'pulse';
  return null;
}

// The base's edge-strip glow: its first LED's colour at its brightness, or null when it is off or unknown.
export function baseGlow(record: StoredLighting | undefined): { colour: string; brightness: number; motion: LightMotion | null } | null {
  const led = record && record.kind === 'led' ? record.leds[0] : undefined;
  if (!led || led.effect === 0 || led.brightness <= 0) return null;
  return { colour: led.colour, brightness: led.brightness, motion: lightMotion(record, 1) };
}

// The stored look the controls start from: a specific LED's own look, LED 0's
// for "All LEDs", or the dial's backlight and colour.
export function controlsFromRecord(record: StoredLighting | undefined, kind: 'dial' | 'led', led: number | null): { backlight: number; colour: string; effect: number } | null {
  if (kind === 'dial') return record && record.kind === 'dial' ? { backlight: record.backlight, colour: record.colour, effect: 0 } : null;
  const look = record && record.kind === 'led' ? record.leds[led ?? 0] : undefined;
  return look ? { backlight: look.brightness, colour: look.colour, effect: look.effect } : null;
}

// Paths that appeared since the last inventory: everything on the first load,
// then only additions, so a replug is answered but steady state writes nothing.
export function appearedTargets(previous: string[] | null, devices: LightingDevice[]): LightingDevice[] {
  if (previous === null) return devices;
  const before = new Set(previous);
  return devices.filter(device => !before.has(device.path_hex));
}

// Sends every stored look of the newly appeared units again, in order. A unit
// a run is driving is skipped (the run owns it), and a failed write is left to
// the next replug or the next change on the Lighting page — this runs in the
// background, so it never surfaces an error of its own. Returns the number of
// units restored.
export async function restoreLighting(store: {
  getState(): LightingSnapshot;
}, targets: LightingDevice[], invoke: Invoke, isHeld: (device: LightingDevice) => boolean): Promise<number> {
  const records = store.getState().records;
  let restored = 0;
  for (const device of targets) {
    const record = records[device.path_hex];
    if (!record || isHeld(device)) continue;
    try {
      if (record.kind === 'dial') {
        await applyLighting(invoke, { targets: [device.path_hex], backlight: record.backlight, colour: record.colour });
      } else {
        for (const [index, look] of record.leds.entries()) {
          if (!look) continue;
          await applyLedLighting(invoke, { targets: [device.path_hex], led: index, colour: look.colour, effect: look.effect, brightness: look.brightness });
        }
      }
      restored++;
    } catch { /* best effort; the page shows its own errors on a real apply */ }
  }
  return restored;
}

export class LightingStateStore {
  private snapshot: LightingSnapshot;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly storage: Pick<Storage, 'getItem' | 'setItem'> | null) {
    this.snapshot = snapshotOf(readLightingRecords(storage), {});
  }

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly getState = (): LightingSnapshot => this.snapshot;

  // Records what an apply (or a reset, with `undefined`) left on each unit.
  record(paths: string[], make: (existing: StoredLighting | undefined) => StoredLighting | undefined) {
    const records = { ...this.snapshot.records };
    let changed = false;
    for (const path of paths) {
      const next = make(records[path]);
      if (next === undefined) { if (path in records) { delete records[path]; changed = true; } continue; }
      if (records[path] === next) continue;
      // A write moves its record to the end, the newest place.
      delete records[path];
      records[path] = next;
      changed = true;
    }
    if (!changed) return;
    this.snapshot = snapshotOf(newest(records), { ...this.snapshot.reported });
    writeLightingRecords(this.storage, this.snapshot.records);
    this.listeners.forEach(listener => listener());
  }

  // Notes what a unit reports about its LEDs; the art uses it only where this app has stored nothing.
  setReported(path: string, report: LedReadback) {
    this.snapshot = snapshotOf({ ...this.snapshot.records }, { ...this.snapshot.reported, [path]: report });
    this.listeners.forEach(listener => listener());
  }

  // Forgets the records of units that are connected now as a module the record can never be applied to: the path
  // answers as another kind (a module swapped on the same port) or an LED module of another size. A path that is not
  // connected keeps its record (a replug on the same port finds its look), and so does a same-kind unit. Returns the
  // paths forgotten.
  forgetUnappliable(devices: LightingDevice[]): string[] {
    const forgotten = devices.filter(device => {
      const record = this.snapshot.records[device.path_hex];
      return record !== undefined && !recordFits(record, device.module);
    }).map(device => device.path_hex);
    if (forgotten.length > 0) this.record(forgotten, () => undefined);
    return forgotten;
  }
}

// Reads the stored records, keeping only the well-formed ones: a known kind,
// normalised colours, effect 0–9, brightness and backlight 0–100, at most five
// LEDs. Anything else (a schema this build never wrote) is dropped.
export function readLightingRecords(storage: Pick<Storage, 'getItem'> | null): Record<string, StoredLighting> {
  try {
    const parsed: unknown = JSON.parse(storage?.getItem(lightingStateStorageKey) ?? '{}');
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {};
    const records: Record<string, StoredLighting> = {};
    for (const [path, value] of Object.entries(parsed)) {
      if (typeof path !== 'string' || path.length === 0) continue;
      const record = parseRecord(value);
      if (record) records[path] = record;
    }
    return newest(records);
  } catch { return {}; }
}

function parseRecord(value: unknown): StoredLighting | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const entry = value as Record<string, unknown>;
  if (entry.kind === 'dial') {
    const backlight = entry.backlight;
    const colour = normaliseHexColour(typeof entry.colour === 'string' ? entry.colour : '');
    if (typeof backlight !== 'number' || !Number.isFinite(backlight) || colour === null) return undefined;
    return { kind: 'dial', backlight: clampBacklight(backlight), colour };
  }
  if (entry.kind !== 'led' || !Array.isArray(entry.leds)) return undefined;
  const leds: (StoredLed | null)[] = entry.leds.slice(0, 5).map(item => {
    if (typeof item !== 'object' || item === null) return null;
    const look = item as Record<string, unknown>;
    const colour = normaliseHexColour(typeof look.colour === 'string' ? look.colour : '');
    const effect = look.effect;
    const brightness = look.brightness;
    if (colour === null || typeof effect !== 'number' || !Number.isFinite(effect)
      || typeof brightness !== 'number' || !Number.isFinite(brightness)) return null;
    return {
      colour,
      effect: Math.min(9, Math.max(0, Math.round(effect))),
      brightness: clampBacklight(brightness),
    };
  });
  return { kind: 'led', leds };
}

export function writeLightingRecords(storage: Pick<Storage, 'setItem'> | null, records: LightingSnapshot['records']): void {
  try { storage?.setItem(lightingStateStorageKey, JSON.stringify(records)); } catch { /* the record stays for this session only */ }
}

// The store components fall back to when none is passed (tests, previews):
// nothing is stored and nothing lights up.
export const emptyLightingState = new LightingStateStore(null);
