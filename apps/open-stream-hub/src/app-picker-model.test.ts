import { describe, expect, it } from 'vitest';
import {
  appIconBatchLimit, appListKey, appMatchRank, appOptions, appSections, closedAppList, createAppIconStore, openAppList, queryAppList,
  readRecentApps, recentAppLimit, recentAppsStorageKey, recordRecentApp, triggerKey,
} from './app-picker-model';
import type { StorageLike } from './action-picker-model';
import type { Invoke } from './profile-session';

const app = (bundle_id: string, name: string) => ({ bundle_id, name });
const finder = app('com.apple.finder', 'Finder');
const music = app('com.apple.Music', 'Music');
const code = app('com.microsoft.VSCode', 'Visual Studio Code');
const safari = app('com.apple.Safari', 'Safari');
const names = (apps: { name: string }[]) => apps.map(item => item.name);

const memoryStorage = (initial: Record<string, string> = {}): StorageLike & { store: Map<string, string> } => {
  const store = new Map(Object.entries(initial));
  return { store, getItem: key => store.get(key) ?? null, setItem: (key, value) => { store.set(key, value); } };
};

describe('app list sections', () => {
  it('lists running apps by name, then recent ones that are not running, with the stored app first among them', () => {
    const sections = appSections([music, finder], [finder, safari], app('com.example.Old', 'Old'), '');
    expect(sections.map(section => [section.label, names(section.apps)])).toEqual([['Running now', ['Finder', 'Music']], ['Recent', ['Old', 'Safari']]]);
    expect(names(appOptions(sections))).toEqual(['Finder', 'Music', 'Old', 'Safari']);
    // A stored app already in the recent list is listed once; empty sections are left out.
    expect(appSections([], [safari], app('COM.APPLE.SAFARI', 'Safari'), '').map(section => names(section.apps))).toEqual([['Safari']]);
    expect(appSections([], [], null, '')).toEqual([]);
  });

  it('filters by name or bundle ID, prefix matches first, then word starts', () => {
    expect(appMatchRank(code, 'vis')).toBe(0);
    expect(appMatchRank(code, 'stu')).toBe(1);
    expect(appMatchRank(code, 'microsoft')).toBe(2);
    expect(appMatchRank(code, 'zz')).toBeNull();
    expect(appMatchRank(code, '  ')).toBe(0);
    const apps = [app('a.one', 'Logic Pro'), app('a.two', 'Pro Tools'), app('a.three', 'Reaper Pro')];
    expect(names(appOptions(appSections(apps, [], null, 'PRO')))).toEqual(['Pro Tools', 'Logic Pro', 'Reaper Pro']);
    expect(appSections([finder, music], [], null, 'nothing')).toEqual([]);
  });
});

describe('app list keyboard', () => {
  const open = { open: true, query: '', active: 0 };

  it('opens on the stored app, or the first row, or searching for a typed character', () => {
    expect(openAppList([finder, music], ' com.apple.music ')).toEqual({ open: true, query: '', active: 1 });
    expect(openAppList([finder, music], 'com.example.none')).toEqual({ open: true, query: '', active: 0 });
    expect(openAppList([], '')).toEqual({ open: true, query: '', active: 0 });
    expect(openAppList([finder, music], 'com.apple.Music', 'f').active).toBe(0);
    expect(triggerKey('ArrowDown', false)).toEqual({ query: '' });
    expect(triggerKey('ArrowUp', false)).toEqual({ query: '' });
    expect(triggerKey('s', false)).toEqual({ query: 's' });
    expect(triggerKey('s', true)).toBeNull();
    expect(triggerKey(' ', false)).toBeNull();
    expect(triggerKey('Enter', false)).toBeNull();
    expect(triggerKey('Tab', false)).toBeNull();
    expect(queryAppList('sa')).toEqual({ open: true, query: 'sa', active: 0 });
  });

  it('moves with the arrows (wrapping), Home and End, and chooses with Enter', () => {
    expect(appListKey(open, 'ArrowDown', 3)?.next.active).toBe(1);
    expect(appListKey({ ...open, active: 2 }, 'ArrowDown', 3)?.next.active).toBe(0);
    expect(appListKey(open, 'ArrowUp', 3)?.next.active).toBe(2);
    expect(appListKey({ ...open, active: -1 }, 'ArrowDown', 3)?.next.active).toBe(0);
    expect(appListKey({ ...open, active: -1 }, 'ArrowUp', 3)?.next.active).toBe(2);
    expect(appListKey({ ...open, active: 1 }, 'Home', 3)?.next.active).toBe(0);
    expect(appListKey(open, 'End', 3)?.next.active).toBe(2);
    expect(appListKey({ ...open, query: 'mu' }, 'ArrowDown', 3)?.next.query).toBe('mu');
    expect(appListKey({ ...open, active: 2 }, 'Enter', 3)).toEqual({ next: closedAppList, choose: 2, close: 'refocus', handled: true });
    expect(appListKey({ ...open, active: -1 }, 'Enter', 3)).toBeNull();
    expect(appListKey(open, 'ArrowDown', 0)).toBeNull();
  });

  it('closes on Escape (back to the trigger) and Tab (focus moves on), and leaves other keys to the search field', () => {
    expect(appListKey(open, 'Escape', 3)).toEqual({ next: closedAppList, close: 'refocus', handled: true });
    expect(appListKey(open, 'Tab', 3)).toEqual({ next: closedAppList, close: 'leave', handled: false });
    expect(appListKey(open, 'a', 3)).toBeNull();
    expect(appListKey(open, 'Backspace', 3)).toBeNull();
  });
});

describe('recent apps', () => {
  it('records chosen apps newest first, once each, up to the limit', () => {
    const storage = memoryStorage();
    expect(readRecentApps(storage)).toEqual([]);
    recordRecentApp(storage, finder);
    recordRecentApp(storage, music);
    recordRecentApp(storage, app('COM.APPLE.FINDER', 'Finder'));
    expect(readRecentApps(storage)).toEqual([app('COM.APPLE.FINDER', 'Finder'), music]);
    for (let index = 0; index < 8; index += 1) recordRecentApp(storage, app(`a.${index}`, `App ${index}`));
    expect(readRecentApps(storage)).toHaveLength(recentAppLimit);
    expect(readRecentApps(storage)[0]).toEqual(app('a.7', 'App 7'));
  });

  it('ignores unreadable entries and never throws', () => {
    expect(readRecentApps(memoryStorage({ [recentAppsStorageKey]: '{broken' }))).toEqual([]);
    expect(readRecentApps(memoryStorage({ [recentAppsStorageKey]: JSON.stringify([finder, { bundle_id: 3 }, null, { bundle_id: ' ', name: 'x' }, { ...music, extra: 1 }]) }))).toEqual([finder, music]);
    const failing: StorageLike = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } };
    expect(readRecentApps(failing)).toEqual([]);
    expect(() => recordRecentApp(failing, finder)).not.toThrow();
  });
});

describe('app icon store', () => {
  const recording = (reply: (ids: string[]) => unknown) => {
    const calls: string[][] = [];
    const invoke = (<T,>(command: string, args?: Record<string, unknown>) => {
      expect(command).toBe('app_icons');
      const ids = (args?.request as { bundle_ids: string[] }).bundle_ids;
      calls.push(ids);
      const result = reply(ids);
      return result instanceof Error ? Promise.reject(result) : Promise.resolve(result as T);
    }) as Invoke;
    return { calls, invoke };
  };

  it('asks once per bundle ID, in batches, caches icons and misses, and notifies', async () => {
    const store = createAppIconStore();
    let notified = 0;
    store.subscribe(() => { notified += 1; });
    const { calls, invoke } = recording(ids => ({ icons: Object.fromEntries(ids.map(id => [id, id === 'gone' ? null : `png-${id}`])) }));
    await store.load(invoke, [' a ', 'a', '', 'gone', 'x'.repeat(300)]);
    expect(calls).toEqual([['a', 'gone']]);
    expect(store.icons.get('a')).toBe('png-a');
    expect(store.icons.get('gone')).toBeNull();
    expect(notified).toBe(1);
    expect(store.version()).toBe(1);
    await store.load(invoke, ['a', 'gone']);
    expect(calls).toHaveLength(1);
    const many = Array.from({ length: appIconBatchLimit + 3 }, (_, index) => `app.${index}`);
    await store.load(invoke, many);
    expect(calls.slice(1).map(batch => batch.length)).toEqual([appIconBatchLimit, 3]);
    expect(notified).toBe(3);
  });

  it('does not ask again while a batch is pending, and keeps placeholders when the backend fails', async () => {
    const store = createAppIconStore();
    const { calls, invoke } = recording(() => new Error('unknown command'));
    const first = store.load(invoke, ['a']);
    const second = store.load(invoke, ['a']);
    await Promise.all([first, second]);
    expect(calls).toEqual([['a']]);
    expect(store.icons.get('a')).toBeNull();
    const odd = createAppIconStore();
    await odd.load(recording(() => ({ icons: { a: 42, b: '' } })).invoke, ['a', 'b', 'c']);
    expect([odd.icons.get('a'), odd.icons.get('b'), odd.icons.get('c')]).toEqual([null, null, null]);
  });
});
