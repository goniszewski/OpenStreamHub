import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { moduleAssignments } from './binding-model';
import { ModuleAssignmentsChip } from './ModuleAssignments';
import type { Profile } from './contracts';

const profile = {
  schema_version: 5, name: 'p',
  bindings: [
    { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
    { module: 'm003k', control: 0, gesture: 'clockwise', action: { type: 'toggle_mute' } },
    { module: 'm003k', control: 2, gesture: 'pressed', action: { type: 'toggle_mute' } },
    { module: 'm005s', control: 1, gesture: 'position_changed', action: { type: 'toggle_mute' } },
  ],
} as unknown as Profile;

describe('moduleAssignments', () => {
  it('counts controls with at least one binding, not bindings, and lists them in control order', () => {
    const result = moduleAssignments(profile, 'm003k');
    expect(result).toMatchObject({ assigned: 2, total: 3, noun: 'knobs' });
    expect(result.entries.map(entry => entry.control)).toEqual([0, 0, 2]);
  });
  it('ignores other modules and says so when nothing is assigned', () => {
    expect(moduleAssignments(profile, 'm032le')).toMatchObject({ assigned: 0, total: 2, noun: 'rollers', entries: [] });
    expect(moduleAssignments(profile, 'm131e').noun).toBe('encoder');
  });
});

describe('moduleAssignments on a keypad', () => {
  const keypad = {
    schema_version: 5, name: 'p', bindings: [],
    keypad: { pages: [
      { id: 'root', name: 'Root', slots: [{ type: 'action', label: 'A', bindings: [{ gesture: 'pressed', action: { type: 'toggle_mute' } }] }, { type: 'folder', page_id: 'f1' }, ...Array(13).fill(null)] },
      { id: 'f1', name: 'Scenes', slots: Array(15).fill(null) },
    ] },
  } as unknown as Profile;
  it('counts a folder key as in use and lists it', () => {
    const result = moduleAssignments(keypad, 'm4315', 'root');
    expect(result).toMatchObject({ assigned: 2, total: 15, folders: [{ control: 1, name: 'Scenes' }] });
    const html = renderToStaticMarkup(<ModuleAssignmentsChip assignments={result} moduleName="Keypad" />);
    expect(html).toContain('2 of 15 keys assigned');
  });
  it('leaves the reserved Back key out of a folder page', () => {
    expect(moduleAssignments(keypad, 'm4315', 'f1')).toMatchObject({ assigned: 0, total: 14 });
  });
});

describe('ModuleAssignmentsChip', () => {
  it('shows x of y and keeps the list closed until pressed', () => {
    const html = renderToStaticMarkup(<ModuleAssignmentsChip assignments={moduleAssignments(profile, 'm003k')} moduleName="3-knob module" />);
    expect(html).toContain('2 of 3 knobs assigned');
    expect(html).toContain('aria-haspopup="dialog"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain('role="dialog"');
  });
});
