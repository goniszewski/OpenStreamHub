import type { Binding, BindingAction, LiveDevice, Profile } from './contracts';
import { controlCounts, gestures, modules } from './ui-model';
import type { ModuleId } from './ui-model';

// A rig is the user's arrangement of module instances, left to right. It is view state
// kept in the webview, not part of the profile, and it never reaches the device.
export interface RigItem { id: string; module: ModuleId }

export const rigStorageKey = 'open-stream-hub.rig';
export const maxRigItems = 8;
export const defaultFaderPosition = 50;
export const defaultRig: RigItem[] = [
  { id: 'default-m4315', module: 'm4315' }, { id: 'default-m003k', module: 'm003k' }, { id: 'default-m131e', module: 'm131e' },
];

export const deviceRigPrefix = 'device:';
export const isDeviceItem = (item: RigItem) => item.id.startsWith(deviceRigPrefix);

const known = new Set<string>(modules.map(module => module.id));

export function addRigModule(rig: RigItem[], module: ModuleId, id: string): RigItem[] {
  return rig.length >= maxRigItems ? rig : [...rig, { id, module }];
}

// Mirrors connected devices into the rig: vanished device items go, new ones are appended, everything else
// (manual items, order) is untouched. Returns the same array when nothing changes.
export function syncDeviceRig(rig: RigItem[], devices: LiveDevice[]): RigItem[] {
  const connected = devices.filter(device => known.has(device.module));
  const present = new Set(connected.map(device => `${deviceRigPrefix}${device.path_hex}`));
  const kept = rig.filter(item => !isDeviceItem(item) || present.has(item.id));
  const have = new Set(kept.map(item => item.id));
  const added = connected.filter(device => !have.has(`${deviceRigPrefix}${device.path_hex}`)).map((device): RigItem => ({ id: `${deviceRigPrefix}${device.path_hex}`, module: device.module }));
  return kept.length === rig.length && !added.length ? rig : [...kept, ...added];
}

export function removeRigModule(rig: RigItem[], id: string): RigItem[] {
  return rig.filter(item => item.id !== id);
}

export function moveRigModule(rig: RigItem[], id: string, offset: -1 | 1): RigItem[] {
  const from = rig.findIndex(item => item.id === id);
  const to = from + offset;
  if (from < 0 || to < 0 || to >= rig.length) return rig;
  const next = [...rig];
  [next[from], next[to]] = [next[to], next[from]];
  return next;
}

export function readRig(storage: Pick<Storage, 'getItem'>): RigItem[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(rigStorageKey) ?? 'null');
    if (!Array.isArray(parsed)) return defaultRig;
    // Device items mirror hardware for one session only; stale ones are dropped on read.
    const stored = parsed.filter(item => !(typeof item?.id === 'string' && item.id.startsWith(deviceRigPrefix)));
    const items = stored.filter((item): item is RigItem => typeof item?.id === 'string' && known.has(item?.module)).slice(0, maxRigItems);
    return items.length === stored.length ? items : defaultRig;
  } catch { return defaultRig; }
}

export function writeRig(storage: Pick<Storage, 'setItem'>, rig: RigItem[]) {
  try { storage.setItem(rigStorageKey, JSON.stringify(rig.filter(item => !isDeviceItem(item)))); } catch { /* the rig is a convenience; keep working without it */ }
}

// The action a control shows on the illustration: its first binding in gesture order.
export function controlAction(profile: Profile, module: ModuleId, control: number): BindingAction | undefined {
  const order = gestures[module];
  const matching = profile.bindings.filter((binding: Binding) => binding.module === module && binding.control === control && order.includes(binding.gesture));
  matching.sort((a, b) => order.indexOf(a.gesture) - order.indexOf(b.gesture));
  return matching[0]?.action;
}

export function boundControlCount(profile: Profile, module: ModuleId): number {
  return Array.from({ length: controlCounts[module] }, (_, control) => control).filter(control => controlAction(profile, module, control)).length;
}

// Maps a pointer position on a vertical fader track (0 = top) to the 0..maximum scale (0 = bottom).
export function faderFromPointer(offsetY: number, trackHeight: number, maximum: number): number {
  if (!(trackHeight > 0)) return 0;
  const ratio = 1 - Math.min(Math.max(offsetY / trackHeight, 0), 1);
  return Math.round(ratio * maximum);
}
