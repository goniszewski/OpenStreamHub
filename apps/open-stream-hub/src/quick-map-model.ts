// The Quick map's data: what every control of the open profile does, as numbered legend lines. Pure, so it is tested
// without a device or a DOM. The wording of actions comes from actionLabel (ui-model.ts); nothing here describes an action itself.
import type { Gesture, Profile, ServiceModule } from './contracts';
import { english } from './i18n-react';
import { keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { actionLabel, controlCounts, controlLabel, gestureLabels, gestures } from './ui-model';

// The order the modules are drawn in.
export const mapModuleOrder: ReadonlyArray<ServiceModule> = ['m4315', 'm131e', 'm003k', 'm005s', 'm032le'];

// One thing a control does: on a gesture (null when the control has only one natural gesture, such as a key press or a fader position).
export interface MapPart { gesture: string | null; action: string }
export interface MapControl {
  module: ServiceModule;
  control: number;
  name: string;
  // The legend number; null for a control with nothing bound (it is drawn without a callout).
  number: number | null;
  // The label a key shows (keypad only).
  keyLabel?: string;
  // The folder a keypad key opens, with the number of keys bound inside it.
  folder?: { name: string; keys: number };
  parts: MapPart[];
}
export interface MapModule { module: ServiceModule; title: string; controls: MapControl[] }
export interface LegendLine { number: number; name: string; text: string; folder: boolean }
export interface QuickMap { profileName: string; modules: MapModule[]; legend: LegendLine[] }

export const partText = (part: MapPart) => part.gesture ? `${part.gesture}: ${part.action}` : part.action;

// Gestures worth naming: a key's press, a fader's position and a roller's two directions read naturally without one.
function naturalGesture(module: ServiceModule, list: Gesture[]): boolean {
  return list.length === 1 && (module === 'm4315' || module === 'm005s');
}

export function quickMap(profile: Profile): QuickMap {
  const byModule = new Map<ServiceModule, MapControl[]>();
  let number = 0;
  const finish = (module: ServiceModule, controls: MapControl[]) => {
    for (const control of controls) if (control.parts.length || control.folder) control.number = ++number;
    if (controls.some(control => control.number !== null)) byModule.set(module, controls);
  };

  for (const module of mapModuleOrder) {
    if (module === 'm4315') { finish(module, keypadControls(profile)); continue; }
    if (module === 'm131e') { finish(module, dialControls(profile)); continue; }
    const controls = Array.from({ length: controlCounts[module] }, (_, control): MapControl => ({ module, control, name: controlLabel(module, control), number: null, parts: [] }));
    for (const gesture of gestures[module]) {
      for (const binding of profile.bindings) {
        if (binding.module !== module || binding.gesture !== gesture || binding.control >= controls.length) continue;
        const target = controls[binding.control];
        // An exact serial binding shadows the any-serial one in the service; the map shows one line per gesture.
        if (target.parts.some(part => part.gesture === (naturalGesture(module, gestures[module]) ? null : gestureLabels[gesture]))) continue;
        target.parts.push({ gesture: naturalGesture(module, gestures[module]) ? null : gestureLabels[gesture], action: actionLabel(binding.action) });
      }
    }
    finish(module, controls);
  }

  const modules: MapModule[] = mapModuleOrder.filter(module => byModule.has(module)).map(module => ({ module, title: english(`quickMap.module.${module}`), controls: byModule.get(module)! }));
  const legend = modules.flatMap(item => item.controls).filter(control => control.number !== null).sort((a, b) => a.number! - b.number!).map((control): LegendLine => {
    const folder = control.folder ? english('quickMap.folder', { name: control.folder.name }) : '';
    const named = control.keyLabel && control.keyLabel !== control.name ? `${control.name} (${control.keyLabel})` : control.name;
    return { number: control.number!, name: named, folder: control.folder !== undefined, text: [...control.parts.map(partText), ...(folder ? [folder] : [])].join('; ') };
  });
  return { profileName: profile.name, modules, legend };
}

// The keypad's root page: schema 1 profiles keep their keys in the flat bindings, which the upgrade moves into the root page.
function keypadControls(profile: Profile): MapControl[] {
  let upgraded: Profile;
  try { upgraded = upgradeKeypadProfile(profile); } catch { return []; }
  const pages = upgraded.keypad?.pages ?? [];
  const root = pages.find(page => page.id === keypadRoot) ?? pages[0];
  if (!root) return [];
  return Array.from({ length: controlCounts.m4315 }, (_, control): MapControl => {
    const slot = root.slots[control] ?? null;
    const base: MapControl = { module: 'm4315', control, name: controlLabel('m4315', control), number: null, parts: [] };
    if (slot?.type === 'folder') {
      const page = pages.find(item => item.id === slot.page_id);
      const keys = (page?.slots ?? []).filter(inner => inner !== null).length;
      return { ...base, keyLabel: page?.name, folder: { name: page?.name ?? english('quickMap.folderMark'), keys } };
    }
    if (slot?.type !== 'action') return base;
    const seen = new Set<string>();
    const parts: MapPart[] = [];
    const pressOnly = slot.bindings.every(binding => binding.gesture === 'pressed');
    for (const binding of slot.bindings) {
      if (seen.has(binding.gesture)) continue;
      seen.add(binding.gesture);
      parts.push({ gesture: pressOnly ? null : gestureLabels[binding.gesture], action: actionLabel(binding.action) });
    }
    return { ...base, keyLabel: slot.label, parts };
  });
}

// The dial is one control: each function (or, without functions, each gesture) is a part.
function dialControls(profile: Profile): MapControl[] {
  const base: MapControl = { module: 'm131e', control: 0, name: controlLabel('m131e', 0), number: null, parts: [] };
  const functions = profile.dial_functions;
  if (functions) {
    for (const item of functions) for (const binding of item.bindings) base.parts.push({ gesture: `${item.name} · ${gestureLabels[binding.gesture]}`, action: actionLabel(binding.action) });
    if (functions.length > 1) base.parts.push({ gesture: null, action: english('quickMap.dialCycle') });
  } else {
    for (const gesture of gestures.m131e) {
      const binding = profile.bindings.find(item => item.module === 'm131e' && item.gesture === gesture);
      if (binding) base.parts.push({ gesture: gestureLabels[gesture], action: actionLabel(binding.action) });
    }
  }
  return [base];
}

// Greedy word wrap for the drawing's text, which an SVG does not do by itself. A word longer than the width stays whole.
export function wrapText(text: string, width: number): string[] {
  const lines: string[] = [];
  let line = '';
  for (const word of text.split(/\s+/).filter(Boolean)) {
    if (line && line.length + 1 + word.length > width) { lines.push(line); line = word; } else line = line ? `${line} ${word}` : word;
  }
  if (line) lines.push(line);
  return lines;
}

// A key label cut to fit its tile, by code points (never splits an emoji).
export function clipKeyLabel(label: string, max: number): string {
  const characters = Array.from(label);
  return characters.length > max ? `${characters.slice(0, max - 1).join('')}…` : label;
}
