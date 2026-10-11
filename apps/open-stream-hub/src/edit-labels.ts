import type { Binding, KeypadPage, KeypadSlot, Profile } from './contracts';
import { actionLabel, controlLabel, gestureLabels } from './ui-model';

// One line for what an edit changed, for the undo/redo list: derived from the profile before and after, so every
// way of editing is described the same way. Action text comes from actionLabel (which keeps macro keys out). Nothing
// here is stored: the list lives in memory next to the history and goes with it.

const bindingKey = (binding: Binding) => `${binding.module}:${binding.control}:${binding.gesture}:${binding.serial_number ?? ''}`;
const bindingName = (binding: Binding) => `${controlLabel(binding.module, binding.control)} ${gestureLabels[binding.gesture].toLowerCase()}`;
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const clip = (name: string) => { const characters = Array.from(name.trim()); return characters.length > 28 ? `${characters.slice(0, 28).join('')}…` : name.trim(); };

function slotLabel(slot: KeypadSlot | undefined): string {
  if (!slot) return 'empty';
  if (slot.type === 'folder') return 'a folder';
  const action = slot.bindings[0]?.action;
  return action ? actionLabel(action) : 'no action';
}

// Keys whose slot or artwork changed on one page, as "Key 3" or "3 keys".
function pageChange(before: KeypadPage | undefined, after: KeypadPage | undefined): string | null {
  if (!before && after) return `Added the key page “${clip(after.name)}”`;
  if (before && !after) return `Removed the key page “${clip(before.name)}”`;
  if (!before || !after) return null;
  if (before.name !== after.name) return `Renamed the key page “${clip(before.name)}” to “${clip(after.name)}”`;
  const slots = Math.max(before.slots.length, after.slots.length);
  const changed: number[] = [];
  for (let index = 0; index < slots; index += 1) {
    if (!same(before.slots[index] ?? null, after.slots[index] ?? null) || !same(before.artwork?.[String(index)], after.artwork?.[String(index)])) changed.push(index);
  }
  if (changed.length === 0) return null;
  if (changed.length > 1) return `Changed ${changed.length} keys`;
  const [index] = changed;
  const artworkOnly = same(before.slots[index] ?? null, after.slots[index] ?? null);
  if (artworkOnly) return `Changed the artwork of Key ${index + 1}`;
  return `Key ${index + 1}: ${slotLabel(after.slots[index])}`;
}

function bindingChange(before: Binding[], after: Binding[]): string | null {
  const previous = new Map(before.map(binding => [bindingKey(binding), binding]));
  const next = new Map(after.map(binding => [bindingKey(binding), binding]));
  const added = after.filter(binding => !previous.has(bindingKey(binding)));
  const removed = before.filter(binding => !next.has(bindingKey(binding)));
  const changed = after.filter(binding => previous.has(bindingKey(binding)) && !same(previous.get(bindingKey(binding))!.action, binding.action));
  const total = added.length + removed.length + changed.length;
  if (total === 0) return null;
  if (total > 1) return `Changed ${total} bindings`;
  if (added[0]) return `${bindingName(added[0])}: ${actionLabel(added[0].action)}`;
  if (removed[0]) return `Removed the ${bindingName(removed[0])} binding`;
  return `${bindingName(changed[0])}: ${actionLabel(changed[0].action)}`;
}

// Every part of the profile that differs gets its own phrase; a single phrase reads as itself, several as a count.
export function describeEdit(before: Profile, after: Profile): string {
  const phrases: string[] = [];
  if (before.name !== after.name) phrases.push(`Renamed the profile to “${clip(after.name)}”`);
  const bindings = bindingChange(before.bindings, after.bindings);
  if (bindings) phrases.push(bindings);
  const beforePages = new Map((before.keypad?.pages ?? []).map(page => [page.id, page]));
  const afterPages = new Map((after.keypad?.pages ?? []).map(page => [page.id, page]));
  const pages = [...new Set([...beforePages.keys(), ...afterPages.keys()])]
    .map(id => pageChange(beforePages.get(id), afterPages.get(id))).filter((phrase): phrase is string => phrase !== null);
  if (pages.length === 1) phrases.push(pages[0]);
  else if (pages.length > 1) phrases.push(`Changed ${pages.length} key pages`);
  if (!same(before.dial_functions, after.dial_functions)) phrases.push('Changed the dial functions');
  if (!same(before.dial_display, after.dial_display)) phrases.push('Changed the dial display');
  if (phrases.length === 0) return 'Edited the profile';
  return phrases.length === 1 ? phrases[0] : `${phrases[0]} and ${phrases.length - 1} more change${phrases.length === 2 ? '' : 's'}`;
}
