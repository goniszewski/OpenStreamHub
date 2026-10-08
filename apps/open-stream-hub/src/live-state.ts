import type { DeviceInputPayload, Gesture } from './contracts';
import { controlCounts, faderMaximum } from './ui-model';
import type { ModuleId } from './ui-model';

// Pure reducer for live device input. It only mirrors what the hardware reports; no action runs.
export interface RollerPulse { direction: 'up' | 'down'; tick: number }
export interface ModuleLive {
  faders: Record<number, number>;
  keysDown: number[];
  angles: Record<number, number>;
  pulses: Record<number, RollerPulse>;
  lastEventAt: number;
}
export type LiveState = Partial<Record<ModuleId, ModuleLive>>;

export const rotationStep = 15;
export const emptyLive: LiveState = {};
export const resetLive = (): LiveState => emptyLive;

// The needle angle is cumulative and never folded back into 0..360: the pointer animates with a CSS transition, and a jump from
// 345 to 0 would sweep the needle backwards through every angle in between.
const rotationSign: Partial<Record<Gesture, 1 | -1>> = { clockwise: 1, right: 1, counter_clockwise: -1, left: -1 };
const emptyModule: ModuleLive = { faders: {}, keysDown: [], angles: {}, pulses: {}, lastEventAt: 0 };

// A feed payload, or a session's input record that stands for `steps` merged turns or roller steps (absent means one).
export type LiveInputPayload = DeviceInputPayload & { steps?: number };
const stepsOf = (payload: LiveInputPayload) => Number.isInteger(payload.steps) && payload.steps! > 0 ? payload.steps! : 1;

export const livePercent = (raw: number, maximum: number) => Math.min(faderMaximum, Math.max(0, Math.round(raw / maximum * faderMaximum)));

export function wellFormed(payload: unknown): payload is DeviceInputPayload {
  const p = payload as DeviceInputPayload | null;
  const event = p?.event;
  return typeof p?.module === 'string' && p.module in controlCounts && typeof event === 'object' && event !== null &&
    typeof event.gesture === 'string' && Number.isInteger(event.control) && typeof event.timestamp_micros === 'number';
}

export function reduceLive(state: LiveState, payload: unknown): LiveState {
  if (!wellFormed(payload)) return state;
  const steps = stepsOf(payload);
  const { module, event } = payload;
  const { control, gesture } = event;
  if (control < 0 || control >= controlCounts[module]) return state;
  const current = state[module] ?? emptyModule;
  const next: ModuleLive = { ...current, lastEventAt: event.timestamp_micros };
  if (gesture === 'position_changed') {
    const value = event.value;
    if (value?.type !== 'position' || !Number.isFinite(value.raw) || !Number.isFinite(value.maximum) || value.maximum <= 0) return state;
    next.faders = { ...current.faders, [control]: livePercent(value.raw, value.maximum) };
  } else if (gesture === 'pressed') {
    next.keysDown = current.keysDown.includes(control) ? current.keysDown : [...current.keysDown, control].sort((a, b) => a - b);
  } else if (gesture === 'released') {
    next.keysDown = current.keysDown.filter(key => key !== control);
  } else if (gesture === 'up' || gesture === 'down') {
    next.pulses = { ...current.pulses, [control]: { direction: gesture, tick: (current.pulses[control]?.tick ?? 0) + steps } };
  } else if (rotationSign[gesture]) {
    next.angles = { ...current.angles, [control]: (current.angles[control] ?? 0) + rotationSign[gesture]! * rotationStep * steps };
  } else if (gesture !== 'short_press' && gesture !== 'long_press') return state;
  return { ...state, [module]: next };
}

// The feed payload one service `event` record stands for, on the module the session runs on (the record names no module), or
// null. Only well-formed `event` records count. short_press and long_press are host gestures the service derives from a press
// and release it already reported, so they add nothing to the preview and are skipped. Live coalescing in the service merges
// consecutive turns and roller steps of one control into one record (`coalesced_event_count`, unbounded while its action queue
// is full); the record keeps that count as `steps`, so the reducer moves the needle or pulse by all of them in one update.
export function sessionInput(record: unknown, module: ModuleId, serialNumber: string | null, feedId: string): LiveInputPayload | null {
  const r = record as { type?: unknown; event?: Partial<DeviceInputPayload['event']> | null; coalesced_event_count?: unknown } | null;
  if (typeof r !== 'object' || r === null || r.type !== 'event' || typeof r.event !== 'object' || r.event === null) return null;
  const { control, gesture, timestamp_micros, value } = r.event;
  const payload: LiveInputPayload = { feed_id: feedId, module, serial_number: serialNumber, event: { control, gesture, timestamp_micros, ...(value !== undefined ? { value } : {}) } as DeviceInputPayload['event'] };
  if (!wellFormed(payload) || payload.event.gesture === 'short_press' || payload.event.gesture === 'long_press') return null;
  const merged = r.coalesced_event_count;
  const stepped = rotationSign[payload.event.gesture] || payload.event.gesture === 'up' || payload.event.gesture === 'down';
  return stepped && typeof merged === 'number' && Number.isInteger(merged) && merged > 1 ? { ...payload, steps: merged } : payload;
}
