import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { describe, expect, it } from 'vitest';
import { actionDragMime } from './action-picker-model';
import type { PickerActionType } from './action-picker-model';
import { controlDropHandlers } from './DevicesPage';
import { keyDragMime } from './key-drag';
import { KeypadScreen } from './KeypadScreen';
import { createKeypadFolder, editKeypadBinding, emptyKeypadPage, keypadRoot } from './keypad-model';
import { keyMenuItems } from './useDeviceEditor';
import { dropNoticeText, dropProfileEdit, dropTargetState } from './useDeviceEditor';
import type { Binding, Profile } from './contracts';

const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
const bound: Profile = { ...flat, bindings: [
  { module: 'm003k', control: 1, gesture: 'clockwise', action: { type: 'timer', seconds: 300 } },
  { module: 'm003k', control: 2, gesture: 'clockwise', serial_number: 'S1', action: { type: 'toggle_mute' } },
] as Binding[] };
const root = emptyKeypadPage();
root.slots[2] = { type: 'action', label: 'Clip', bindings: [] };
const nested = emptyKeypadPage('sub', 'Sub');
const paged: Profile = { schema_version: 2, name: 'Paged', bindings: [], keypad: { pages: [root, nested] } };
const withFolder = createKeypadFolder(paged, ['root'], 3, 'fx', 'FX');

const dropped = (data: Record<string, string>) => {
  const prevented: boolean[] = [];
  return {
    dataTransfer: { types: [actionDragMime], getData: (name: string) => data[name] ?? '' },
    preventDefault: () => { prevented.push(true); },
    prevented,
  };
};

describe('dropProfileEdit', () => {
  it('assigns the default action to the primary gesture of a flat control', () => {
    const next = dropProfileEdit(flat, [keypadRoot], 'm003k', 2, 'open_url');
    expect(next.bindings).toEqual([{ module: 'm003k', control: 2, gesture: 'clockwise', action: { type: 'open_url', url: 'https://example.com' } }]);
    // The one OBS entry drops the scene operation; the editor's Operation dropdown changes it.
    const obs = dropProfileEdit(flat, [keypadRoot], 'm003k', 2, 'obs');
    expect(obs.bindings).toEqual([{ module: 'm003k', control: 2, gesture: 'clockwise', action: { type: 'obs_scene', scene: 'Scene' } }]);
    const fader = dropProfileEdit(flat, [keypadRoot], 'm005s', 0, 'volume_from_position');
    expect(fader.bindings).toEqual([{ module: 'm005s', control: 0, gesture: 'position_changed', action: { type: 'volume_from_position' } }]);
  });

  it('assigns to the key press of the keypad page being edited and names the key', () => {
    const next = dropProfileEdit(paged, [keypadRoot], 'm4315', 5, 'text');
    const slot = next.keypad!.pages[0].slots[5];
    // A new key is named after its action, as the inspector's picker does.
    expect(slot).toMatchObject({ type: 'action', label: 'Paste “Hello”' });
    expect(slot?.type === 'action' && slot.bindings).toEqual([{ gesture: 'pressed', action: { type: 'text', text: 'Hello', press_enter: false } }]);
    // The existing label of a named key survives the drop.
    const kept = dropProfileEdit(paged, [keypadRoot], 'm4315', 2, 'timer');
    expect(kept.keypad!.pages[0].slots[2]).toMatchObject({ type: 'action', label: 'Clip' });
  });

  it('refuses folder keys and the reserved Back key with the inspector messages', () => {
    expect(() => dropProfileEdit(withFolder, [keypadRoot], 'm4315', 3, 'open_url')).toThrow('Open the folder to configure its keys.');
    expect(() => dropProfileEdit(withFolder, [keypadRoot, 'fx'], 'm4315', 0, 'open_url')).toThrow('The Back key is reserved on folder pages.');
  });
});

describe('dropTargetState and dropNoticeText', () => {
  it('reports a replaced binding in catalogue names', () => {
    const found = dropTargetState(bound, [keypadRoot], 'm003k', 1);
    expect(found.previous).toEqual({ type: 'timer', seconds: 300 });
    expect(found.keptSerial).toBeUndefined();
    expect(dropNoticeText(found, 'open_url', 'Knob 2')).toBe('Replaced Timer with Open URL on Knob 2.');
  });

  it('reports what an exact-serial binding keeps doing when the drop adds alongside it', () => {
    const found = dropTargetState(bound, [keypadRoot], 'm003k', 2);
    expect(found.previous).toBeUndefined();
    expect(found.keptSerial).toBe('S1');
    expect(dropNoticeText(found, 'open_url', 'Knob 3')).toBe('Added Open URL to Knob 3; the binding for serial S1 is unchanged.');
    expect(dropNoticeText(dropTargetState(flat, [keypadRoot], 'm003k', 0), 'open_url', 'Knob 1')).toBe('Added Open URL to Knob 1.');
  });

  it('reads the same state from a keypad key press', () => {
    const withPress: Profile = { ...paged, keypad: { pages: [{ ...root, slots: root.slots.map((slot, at) => at === 2 ? { type: 'action', label: 'Clip', bindings: [
      { gesture: 'pressed' as const, serial_number: 'K1', action: { type: 'toggle_mute' } },
    ] } : slot) }] } };
    const found = dropTargetState(withPress, [keypadRoot], 'm4315', 2);
    expect(found.previous).toBeUndefined();
    expect(found.keptSerial).toBe('K1');
    expect(dropNoticeText(found, 'timer', 'Key 3')).toBe('Added Timer to Key 3; the binding for serial K1 is unchanged.');
  });
});

describe('controlDropHandlers', () => {
  const calls: PickerActionType[][] = [];
  const drop = (type: PickerActionType) => { calls.push([type]); };
  const handlers = controlDropHandlers('m003k', 1, drop);

  it('gives disabled controls no drag handlers', () => {
    expect(controlDropHandlers('m003k', 1, undefined)).toEqual({});
  });

  it('admits only action drags', () => {
    let prevented = 0;
    handlers.onDragOver!({ dataTransfer: { types: ['text/plain'] }, preventDefault: () => { prevented += 1; } } as never);
    expect(prevented).toBe(0);
    handlers.onDragOver!({ dataTransfer: { types: ['text/html', actionDragMime] }, preventDefault: () => { prevented += 1; } } as never);
    expect(prevented).toBe(1);
  });

  it('drops a known action and refuses foreign or module-invalid payloads', () => {
    const good = dropped({ [actionDragMime]: 'obs' });
    handlers.onDrop!(good as never);
    expect(calls).toEqual([['obs']]);
    expect(good.prevented).toEqual([true]);
    handlers.onDrop!(dropped({ [actionDragMime]: 'not-a-type' }) as never);
    handlers.onDrop!(dropped({}) as never);
    // The fader-only action is not valid on a knob.
    handlers.onDrop!(dropped({ [actionDragMime]: 'volume_from_position' }) as never);
    expect(calls).toHaveLength(1);
  });
});

describe('keypad tile drops', () => {
  const walk = (node: ReactNode): ReactElement[] => {
    if (Array.isArray(node)) return node.flatMap(walk);
    if (!isValidElement(node)) return [];
    return [node as ReactElement, ...walk((node.props as { children?: ReactNode }).children)];
  };
  type TileProps = { className?: string; 'aria-label'?: string; onDragOver?: unknown; onDrop?: unknown; draggable?: boolean; onDragStart?: unknown; onContextMenu?: unknown };
  const tiles = (path: string[], onDropAction?: (control: number, type: PickerActionType) => void, rearrange?: {
    move?: (source: { path: string[]; control: number }, to: { path: string[]; control: number }, copy: boolean) => void;
    context?: (control: number, at: { x: number; y: number }) => void;
  }) =>
    walk(KeypadScreen({
      pages: path.length > 1 ? [root, withFolder.keypad!.pages.at(-1)!] : [root], allPages: withFolder.keypad!.pages, path, selected: 1,
      onSelect: () => {}, onNavigate: () => {}, onDropAction, onMoveKey: rearrange?.move, onKeyContext: rearrange?.context,
    }))
      .filter(element => element.type === 'button' && String((element.props as TileProps).className).includes('keypad-tile'))
      .map(element => element.props as TileProps);

  it('accepts action drops on keys but never on the reserved Back key', () => {
    const atRoot = tiles(['root'], () => {});
    expect(atRoot).toHaveLength(15);
    expect(atRoot[4]['aria-label']).toBe('Select Key 5, empty');
    expect(typeof atRoot[4].onDrop).toBe('function');
    const inFolder = tiles(['root', 'fx'], () => {});
    expect(inFolder[0]['aria-label']).toBe('Back one level, L1');
    expect(inFolder[0].onDragOver).toBeUndefined();
    expect(inFolder[0].onDrop).toBeUndefined();
    expect(inFolder[0].onContextMenu).toBeUndefined();
    expect(inFolder[1]['aria-label']).toBe('Select Key 2, empty');
    expect(typeof inFolder[1].onDrop).toBe('function');
  });

  it('passes the key and the payload type to the editor', () => {
    const drops: [number, PickerActionType][] = [];
    const key5 = tiles(['root'], (control, type) => { drops.push([control, type]); })[4];
    (key5 as { onDrop(event: unknown): void }).onDrop(dropped({ [actionDragMime]: 'set_volume' }));
    expect(drops).toEqual([[4, 'set_volume']]);
    // The fader-only action is not valid on a key.
    (key5 as { onDrop(event: unknown): void }).onDrop(dropped({ [actionDragMime]: 'volume_from_position' }));
    expect(drops).toHaveLength(1);
  });

  it('turns drops off entirely without a handler', () => {
    for (const tile of tiles(['root'])) {
      expect(tile.onDragOver).toBeUndefined();
      expect(tile.onDrop).toBeUndefined();
    }
  });

  it('drags occupied keys only, carrying their source', () => {
    const data = new Map<string, string>();
    const occupied = tiles(['root'], undefined, { move: () => {} })[2];
    expect(occupied.draggable).toBe(true);
    expect(typeof occupied.onDragStart).toBe('function');
    (occupied as { onDragStart(event: unknown): void }).onDragStart({ dataTransfer: { setData: (name: string, value: string) => { data.set(name, value); }, effectAllowed: '' } });
    expect(data.get(keyDragMime)).toBe(JSON.stringify({ path: ['root'], control: 2 }));
    // Empty keys and the Back key cannot start a drag.
    expect(tiles(['root'], undefined, { move: () => {} })[4].draggable).toBeUndefined();
    expect(tiles(['root', 'fx'], undefined, { move: () => {} })[0].draggable).toBeUndefined();
    // Without a move handler nothing is draggable.
    expect(tiles(['root'])[2].draggable).toBeUndefined();
  });

  it('drops a dragged key as a move, a swap or an Alt copy, and falls through to action drops', () => {
    const moves: [string[], number, number, boolean][] = [];
    const key5 = tiles(['root'], undefined, { move: (source, to, copy) => { moves.push([source.path, source.control, to.control, copy]); } })[4];
    const keyDrop = (alt: boolean) => ({
      altKey: alt,
      dataTransfer: { types: [keyDragMime], getData: (name: string) => name === keyDragMime ? JSON.stringify({ path: ['root'], control: 2 }) : '' },
      preventDefault: () => {},
    });
    (key5 as { onDrop(event: unknown): void }).onDrop(keyDrop(false));
    expect(moves).toEqual([[['root'], 2, 4, false]]);
    (key5 as { onDrop(event: unknown): void }).onDrop(keyDrop(true));
    expect(moves).toEqual([[['root'], 2, 4, false], [['root'], 2, 4, true]]);
    // An action payload still assigns the action.
    (key5 as { onDrop(event: unknown): void }).onDrop(dropped({ [actionDragMime]: 'set_volume' }));
    expect(moves).toHaveLength(2);
    // A foreign key payload is ignored entirely.
    (key5 as { onDrop(event: unknown): void }).onDrop({ altKey: false, dataTransfer: { types: [keyDragMime], getData: () => 'junk' }, preventDefault: () => {} });
    expect(moves).toHaveLength(2);
  });

  it('right-clicks a key into the Copy/Paste/Clear menu, never on the Back key', () => {
    const contexts: [number, number, number][] = [];
    const key5 = tiles(['root'], undefined, { context: (control, at) => { contexts.push([control, at.x, at.y]); } })[4];
    expect(typeof key5.onContextMenu).toBe('function');
    (key5 as { onContextMenu(event: unknown): void }).onContextMenu({ preventDefault: () => {}, clientX: 30, clientY: 40 });
    expect(contexts).toEqual([[4, 30, 40]]);
    // Without a context handler the tile does not prevent the browser menu.
    expect(tiles(['root'])[4].onContextMenu).toBeUndefined();
  });
});

describe('key tile context menu items', () => {
  type Stub = Parameters<typeof keyMenuItems>[0];
  const stub = (slot: unknown, clipboard: unknown): Stub => ({
    keyMenu: { control: 4, x: 10, y: 12 }, currentKeypadPage: { id: 'root', name: 'Root', slots: root.slots.map((entry, at) => at === 4 ? slot : entry) },
    clipboard, copyKey: () => {}, pasteKey: () => {}, clearKey: () => {},
  } as unknown as Stub);
  const copiedKey = { kind: 'key', key: { type: 'action', slot: { type: 'action', label: 'Clip', bindings: [] } } };
  const items = (tile: unknown, clipboard: unknown) => Object.fromEntries(keyMenuItems(stub(tile, clipboard), true).map(item => [item.label, item]));

  it('enables Copy and Clear only for occupied keys, Paste only for an empty key with a copied key', () => {
    expect(items({ type: 'action', label: 'Clip', bindings: [] }, copiedKey)).toMatchObject({ Copy: { disabled: false }, Paste: { disabled: true }, 'Clear key': { disabled: false } });
    expect(items(null, copiedKey)).toMatchObject({ Copy: { disabled: true }, Paste: { disabled: false }, 'Clear key': { disabled: true } });
    expect(items(null, undefined).Paste.disabled).toBe(true);
    // A copied binding cannot paste onto a key.
    expect(items(null, { kind: 'binding' }).Paste.disabled).toBe(true);
    // A folder reads Delete folder; shortcuts follow the platform hint format.
    expect(items({ type: 'folder', page_id: 'fx' }, undefined)['Delete folder']).toBeTruthy();
    expect(keyMenuItems(stub({ type: 'action', label: 'Clip', bindings: [] }, copiedKey), true).map(item => item.hint)).toEqual(['⌘C', '⌘V', undefined]);
  });
});
