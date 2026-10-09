import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ActionPicker, closedPicker, cycleEntryFocus, focusTrigger } from './ActionPicker';
import { actionDragMime, entriesForScope } from './action-picker-model';
import type { PickerActionType, StorageLike } from './action-picker-model';
import type { BindingAction } from './contracts';

const memoryStorage = (): StorageLike & { written(): string[] } => {
  const recent: string[] = [];
  return { written: () => [...recent], getItem: () => null, setItem: (_key, value) => { recent.push(value); } };
};

// Direct element-tree helpers: ActionPicker is hook-free, so tests call it as a function and
// invoke the handlers exactly as the browser would.
type AnyElement = ReactElement<{ [key: string]: unknown }>;

function elementsOf(node: ReactNode): AnyElement[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(elementsOf);
  if (isValidElement(node)) return [node as AnyElement, ...elementsOf((node.props as { children?: ReactNode }).children)];
  return [];
}

const treeOf = (props: Parameters<typeof ActionPicker>[0]) => ActionPicker(props) as AnyElement;
type Props = Parameters<typeof ActionPicker>[0];

const buttons = (tree: AnyElement) => elementsOf(tree).filter(element => element.type === 'button');
const findButton = (tree: AnyElement, test: (props: Record<string, unknown>) => boolean) => {
  const found = buttons(tree).filter(element => test(element.props));
  expect(found).toHaveLength(1);
  return found[0];
};

const base = (over: Partial<Props> = {}): Props => ({
  state: { open: true, query: '' }, onStateChange: () => {}, scope: 'binding', action: null,
  triggerId: 'binding-action', onAssign: () => {}, recent: [], ...over,
});

describe('closed picker', () => {
  it('shows the trigger only, labelled with the bound action', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base({ state: closedPicker })));
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).toContain('Choose an action');
    expect(markup).toContain('aria-haspopup="dialog"');
    expect(markup).not.toContain('role="dialog"');
    expect(markup).not.toContain('Search actions');
    const bound = renderToStaticMarkup(createElement(ActionPicker, base({ state: closedPicker, action: { type: 'open_url', url: 'https://example.com' } })));
    expect(bound).toContain('aria-expanded="false"');
    expect(bound).toContain('>Open URL</span>');
    expect(bound).not.toContain('https://example.com');
    // A bound obs_* action reads as the one OBS entry, leaving the operation to the editor.
    const obs = renderToStaticMarkup(createElement(ActionPicker, base({ state: closedPicker, action: { type: 'obs_source_visibility', scene: 'L', source: 'S', op: 'off' } })));
    expect(obs).toContain('>OBS</span>');
    expect(obs).not.toContain('Source visibility</span>');
  });

  it('opens on a trigger press', () => {
    let next: Props['state'] | null = null;
    const tree = treeOf(base({ state: closedPicker, onStateChange: state => { next = state; } }));
    (findButton(tree, props => props.id === 'binding-action').props as { onClick(): void }).onClick();
    expect(next).toEqual({ open: true, query: '' });
  });
});

describe('open panel', () => {
  it('lists the catalogue grouped by category', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base()));
    expect(markup).toContain('role="dialog"');
    expect(markup).toContain('aria-label="Choose an action"');
    expect(markup).toContain('id="binding-action-search"');
    for (const label of ['Audio', 'Keyboard', 'Apps &amp; web', 'Media', 'System', 'OBS', 'Timers &amp; sequences', 'Navigation']) expect(markup).toContain(`>${label}</p>`);
    for (const type of ['open_url', 'text', 'media', 'system', 'multi_action', 'shortcut', 'profile', 'timer', 'obs', 'volume']) expect(markup).toContain(`data-action-type="${type}"`);
    expect(markup).not.toContain('data-action-type="volume_from_position"');
    expect(markup).not.toContain('data-action-type="unbound"');
  });

  it('marks the bound action and offers faders only the position action', () => {
    const current = renderToStaticMarkup(createElement(ActionPicker, base({ action: { type: 'media', command: 'stop' } })));
    expect(current).toContain('data-action-type="media"');
    expect(current).toMatch(/class="picker-entry current" data-action-entry="" data-action-type="media"/);
    expect((current.match(/aria-current="true"/g) ?? []).length).toBe(1);
    // The one OBS entry is the current one for every bound obs_* action.
    const obs = renderToStaticMarkup(createElement(ActionPicker, base({ action: { type: 'obs_scene', scene: 'Live' } })));
    expect((obs.match(/aria-current="true"/g) ?? []).length).toBe(1);
    expect(obs).toMatch(/class="picker-entry current" data-action-entry="" data-action-type="obs"/);
    const fader = renderToStaticMarkup(createElement(ActionPicker, base({ scope: 'binding-fader', action: { type: 'volume_from_position' } })));
    expect(fader).toContain('data-action-type="volume_from_position"');
    expect(fader).toContain('data-action-type="unbound"');
    expect(fader).not.toContain('data-action-type="open_url"');
  });

  it('shows the fader trigger label when bound', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base({ scope: 'binding-fader', state: closedPicker, action: { type: 'volume_from_position' } })));
    expect(markup).toContain('>Output volume follows position</span>');
  });
});

describe('search', () => {
  it('narrows the catalogue and reports a miss', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base({ state: { open: true, query: 'obs' } })));
    expect(markup).toContain('data-action-type="obs"');
    expect(markup).not.toContain('data-action-type="obs_scene"');
    expect(markup).not.toContain('data-action-type="open_url"');
    expect(markup).not.toContain('>Recent</p>');
    const miss = renderToStaticMarkup(createElement(ActionPicker, base({ state: { open: true, query: 'printer' } })));
    expect(miss).toContain('No action matches “printer”.');
    expect(miss).not.toContain('data-action-type=');
  });

  it('reports every keystroke as the new query', () => {
    let next: Props['state'] | null = null;
    const tree = treeOf(base({ onStateChange: state => { next = state; } }));
    const input = elementsOf(tree).find(element => element.type === 'input' && (element.props as { id?: string }).id === 'binding-action-search');
    (input!.props as { onChange(event: { target: { value: string } }): void }).onChange({ target: { value: 'obs' } });
    expect(next).toEqual({ open: true, query: 'obs' });
  });
});

describe('recent actions', () => {
  it('lists in-memory recents that fit the scope, newest first', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base({ recent: ['timer', 'open_url', 'volume_from_position' as PickerActionType] })));
    expect(markup).toContain('aria-label="Recently used actions"');
    const timer = markup.indexOf('data-action-type="timer"');
    const url = markup.indexOf('data-action-type="open_url"');
    const section = markup.indexOf('aria-label="Recently used actions"');
    expect(timer).toBeGreaterThan(section);
    expect(url).toBeGreaterThan(timer);
    // A recent entry valid on other modules is not offered where it does not fit.
    expect(markup).not.toContain('data-action-type="volume_from_position"');
  });

  it('hides the recent section while searching', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base({ recent: ['timer'], state: { open: true, query: 'timer' } })));
    expect(markup).not.toContain('aria-label="Recently used actions"');
    expect(markup).toContain('data-action-type="timer"');
  });
});

describe('click-assign', () => {
  it('assigns the chosen type, records it as recent and closes', () => {
    const assigned: (PickerActionType | null)[] = [];
    const states: Props['state'][] = [];
    const storage = memoryStorage();
    const tree = treeOf(base({ onAssign: type => { assigned.push(type); }, onStateChange: state => { states.push(state); }, storage }));
    (findButton(tree, props => props['data-action-type'] === 'open_url').props as { onClick(): void }).onClick();
    expect(assigned).toEqual(['open_url']);
    expect(states).toEqual([closedPicker]);
    expect(storage.written()).toEqual([JSON.stringify(['open_url'])]);
  });

  it('unassigns without recording a recent entry', () => {
    const assigned: (PickerActionType | null)[] = [];
    const states: Props['state'][] = [];
    const storage = memoryStorage();
    const tree = treeOf(base({ action: { type: 'volume', delta: 5 }, onAssign: type => { assigned.push(type); }, onStateChange: state => { states.push(state); }, storage }));
    (findButton(tree, props => props['data-action-type'] === 'unbound').props as { onClick(): void }).onClick();
    expect(assigned).toEqual([null]);
    expect(states).toEqual([closedPicker]);
    expect(storage.written()).toEqual([]);
  });

  it('offers no unassign inside a multi-action step', () => {
    const markup = renderToStaticMarkup(createElement(ActionPicker, base({ scope: 'step', action: { type: 'toggle_mute' } })));
    expect(markup).not.toContain('data-action-type="unbound"');
    expect(markup).not.toContain('data-action-type="multi_action"');
    expect(markup).not.toContain('data-action-type="timer"');
  });
});

describe('drag-assign', () => {
  it('writes the action type as the drag payload', () => {
    const stored: Record<string, string> = {};
    const tree = treeOf(base());
    const entry = findButton(tree, props => props['data-action-type'] === 'obs');
    expect(entry.props.draggable).toBe(true);
    const handler = entry.props.onDragStart as ((event: { dataTransfer: unknown }) => void) | undefined;
    expect(handler).toBeTypeOf('function');
    handler!({ dataTransfer: { setData: (name: string, value: string) => { stored[name] = value; }, effectAllowed: '' } });
    expect(stored[actionDragMime]).toBe('obs');
  });

  it('carries no payload in a multi-action step', () => {
    const tree = treeOf(base({ scope: 'step', action: { type: 'toggle_mute' }, allowDrag: false }));
    const entry = findButton(tree, props => props['data-action-type'] === 'open_url');
    expect(entry.props.draggable).toBe(false);
    expect(entry.props.onDragStart).toBeUndefined();
  });
});

describe('keyboard accessibility', () => {
  it('closes on Escape', () => {
    let next: Props['state'] | null = null;
    let prevented = false;
    const tree = treeOf(base({ onStateChange: state => { next = state; } }));
    const panel = elementsOf(tree).find(element => element.type === 'div' && (element.props as { role?: string }).role === 'dialog');
    (panel!.props as { onKeyDown(event: { key: string; preventDefault(): void }): void }).onKeyDown({ key: 'Escape', preventDefault: () => { prevented = true; } });
    expect(next).toEqual(closedPicker);
    expect(prevented).toBe(true);
  });

  const fakeRoot = (labels: string[]) => {
    const focused: string[] = [];
    const entries = labels.map(label => ({ label, focus: () => { focused.push(label); } }));
    return { root: { querySelectorAll: () => entries }, focused, entries };
  };

  it('moves focus with the arrow keys, wrapping at both ends', () => {
    const page = fakeRoot(['Audio entry', 'Keyboard entry', 'OBS entry']);
    cycleEntryFocus(page.root as unknown as ParentNode, null, 1);
    expect(page.focused).toEqual(['Audio entry']);
    cycleEntryFocus(page.root as unknown as ParentNode, page.entries[2] as unknown as Element, 1);
    expect(page.focused).toEqual(['Audio entry', 'Audio entry']);
    cycleEntryFocus(page.root as unknown as ParentNode, page.entries[1] as unknown as Element, -1);
    expect(page.focused).toEqual(['Audio entry', 'Audio entry', 'Audio entry']);
    cycleEntryFocus(page.root as unknown as ParentNode, null, -1);
    expect(page.focused).toEqual(['Audio entry', 'Audio entry', 'Audio entry', 'OBS entry']);
  });

  it('handles an empty list without throwing', () => {
    cycleEntryFocus({ querySelectorAll: () => [] } as unknown as ParentNode, null, 1);
  });

  it('moves panel focus from the dialog keydown', () => {
    const page = fakeRoot(['First', 'Second']);
    const tree = treeOf(base());
    const panel = elementsOf(tree).find(element => element.type === 'div' && (element.props as { role?: string }).role === 'dialog');
    (panel!.props as { onKeyDown(event: { key: string; preventDefault(): void; currentTarget: unknown }): void })
      .onKeyDown({ key: 'ArrowDown', preventDefault: () => {}, currentTarget: page.root });
    expect(page.focused).toEqual(['First']);
  });
});

describe('entry coverage', () => {
  it('offers every binding-scope entry with its label', () => {
    const tree = treeOf(base());
    for (const entry of entriesForScope('binding')) {
      const button = findButton(tree, props => props['data-action-type'] === entry.type);
      expect(elementsOf(button).some(node => node.type === 'span' && (node.props as { children?: string }).children === entry.label), entry.type).toBe(true);
    }
  });
});

describe('focus return', () => {
  afterEach(() => vi.unstubAllGlobals());

  const withTrigger = () => {
    const focused: string[] = [];
    vi.stubGlobal('document', { getElementById: (id: string) => ({ id, focus: () => { focused.push(id); } }) });
    return focused;
  };

  it('focuses the element with the trigger id', () => {
    const focused = withTrigger();
    focusTrigger('step-2-action');
    expect(focused).toEqual(['step-2-action']);
  });

  it('returns focus to the trigger on assign, unassign, Escape and toggle-close', () => {
    const focused = withTrigger();
    const assigned = treeOf(base({ action: { type: 'volume', delta: 5 } }));
    (findButton(assigned, props => props['data-action-type'] === 'open_url').props as { onClick(): void }).onClick();
    (findButton(assigned, props => props['data-action-type'] === 'unbound').props as { onClick(): void }).onClick();
    expect(focused).toEqual(['binding-action', 'binding-action']);
    const escapeTree = treeOf(base());
    (elementsOf(escapeTree).find(element => element.type === 'div' && (element.props as { role?: string }).role === 'dialog')!.props as
      { onKeyDown(event: { key: string; preventDefault(): void }): void }).onKeyDown({ key: 'Escape', preventDefault: () => {} });
    const trigger = treeOf(base({ state: { open: true, query: '' } }));
    (findButton(trigger, props => props.id === 'binding-action').props as { onClick(): void }).onClick();
    expect(focused).toEqual(['binding-action', 'binding-action', 'binding-action', 'binding-action']);
  });
});
