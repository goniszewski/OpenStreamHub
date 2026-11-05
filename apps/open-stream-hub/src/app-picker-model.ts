import type { RunningApp } from './contracts';
import type { StorageLike } from './action-picker-model';
import type { Invoke } from './profile-session';

// The app picker's list: the apps running now and the recently chosen ones that are not, filtered by the search text,
// then the "Other app…" row (the system picker). The keyboard walks one flat option list: every app row in section
// order, then Other app… at index `rows.length`. Pure, so the component only renders what these return.
export interface AppSection { key: 'running' | 'recent'; label: string; apps: RunningApp[] }

const sameId = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

// 0 when the name starts with the query, 1 when a word of it does, 2 when the name or bundle ID contains it, else null.
export function appMatchRank(app: RunningApp, query: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const name = app.name.toLowerCase();
  if (name.startsWith(q)) return 0;
  if (name.split(/[\s\-_.]+/).some(word => word.startsWith(q))) return 1;
  return name.includes(q) || app.bundle_id.toLowerCase().includes(q) ? 2 : null;
}

const filtered = (apps: RunningApp[], query: string) => apps
  .map(app => ({ app, rank: appMatchRank(app, query) }))
  .filter((entry): entry is { app: RunningApp; rank: number } => entry.rank !== null)
  .sort((a, b) => a.rank - b.rank)
  .map(entry => entry.app);

// Running apps by name; recent ones that are not running, newest first, with the stored app first among them when
// neither list has it (`current`, named by its installed name or its ID). Empty sections are left out.
export function appSections(running: RunningApp[], recent: RunningApp[], current: RunningApp | null, query: string): AppSection[] {
  const sorted = [...running].sort((a, b) => a.name.localeCompare(b.name));
  const others = [...(current ? [current] : []), ...recent]
    .filter((app, index, all) => !running.some(item => sameId(item.bundle_id, app.bundle_id)) && all.findIndex(item => sameId(item.bundle_id, app.bundle_id)) === index);
  return ([['running', 'Running now', sorted], ['recent', 'Recent', others]] as const)
    .map(([key, label, apps]) => ({ key, label, apps: filtered(apps, query) }))
    .filter(section => section.apps.length > 0);
}

export const appOptions = (sections: AppSection[]): RunningApp[] => sections.flatMap(section => section.apps);

// The popover: closed, or open with the search text and the active option (-1 none).
export interface AppListState { open: boolean; query: string; active: number }
export const closedAppList: AppListState = { open: false, query: '', active: -1 };

// Opening highlights the stored app when it is listed, else the first row (Other app… when nothing is listed).
export function openAppList(options: RunningApp[], bundleId: string, query = ''): AppListState {
  const index = query ? -1 : options.findIndex(app => sameId(app.bundle_id, bundleId.trim()));
  return { open: true, query, active: Math.max(index, 0) };
}

// Keys on the closed trigger: the arrows open the list, a printable character opens it already searching for it
// (typeahead). null leaves the key to the button (Enter and Space click it).
export function triggerKey(key: string, modified: boolean): { query: string } | null {
  if (modified) return null;
  if (key === 'ArrowDown' || key === 'ArrowUp') return { query: '' };
  return key.length === 1 && key !== ' ' ? { query: key } : null;
}

// Keys in the open list's search field, over `count` options (the app rows plus Other app…). Arrows wrap; Home / End
// jump; Enter chooses the active option; Escape closes; Tab closes and lets focus move on. null: a key for the field.
export type AppListKeyResult = { next: AppListState; choose?: number; close?: 'refocus' | 'leave'; handled: boolean };
export function appListKey(state: AppListState, key: string, count: number): AppListKeyResult | null {
  const at = (active: number) => ({ next: { ...state, active }, handled: true });
  switch (key) {
    case 'ArrowDown': return count ? at(state.active < 0 ? 0 : (state.active + 1) % count) : null;
    case 'ArrowUp': return count ? at(state.active < 0 ? count - 1 : (state.active - 1 + count) % count) : null;
    case 'Home': return count ? at(0) : null;
    case 'End': return count ? at(count - 1) : null;
    case 'Enter': return state.active >= 0 && state.active < count ? { next: closedAppList, choose: state.active, close: 'refocus', handled: true } : null;
    case 'Escape': return { next: closedAppList, close: 'refocus', handled: true };
    case 'Tab': return { next: closedAppList, close: 'leave', handled: false };
    default: return null;
  }
}

// Typing in the search field highlights the best match (the first row), or Other app… when nothing matches.
export const queryAppList = (query: string): AppListState => ({ open: true, query, active: 0 });

// Apps chosen in the picker, newest first, so an app that is not running now is still one click away. A nicety like
// the recent actions: unreadable storage yields an empty list and a failed write is dropped.
export const recentAppsStorageKey = 'open-stream-hub.recent-apps';
export const recentAppLimit = 5;

export function readRecentApps(storage: StorageLike): RunningApp[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(recentAppsStorageKey) ?? 'null');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((app): app is RunningApp => typeof app?.bundle_id === 'string' && typeof app?.name === 'string' && app.bundle_id.trim() !== '')
      .slice(0, recentAppLimit).map(app => ({ bundle_id: app.bundle_id, name: app.name }));
  } catch { return []; }
}

export function recordRecentApp(storage: StorageLike, app: RunningApp): void {
  try {
    const next = [{ bundle_id: app.bundle_id, name: app.name }, ...readRecentApps(storage).filter(item => !sameId(item.bundle_id, app.bundle_id))].slice(0, recentAppLimit);
    storage.setItem(recentAppsStorageKey, JSON.stringify(next));
  } catch { /* The recent list must never block choosing. */ }
}

// App icons for the list rows, from app_icons (LaunchServices; nothing is uploaded), cached per bundle ID for the
// session: a PNG as base64, or null once the app has none or the lookup failed (not asked again). Rows show a
// placeholder until theirs arrives; `version` changes whenever icons arrive, for useSyncExternalStore.
export const appIconBatchLimit = 64;
const maxBundleIdLength = 256;

export interface AppIconStore {
  icons: ReadonlyMap<string, string | null>;
  version(): number;
  subscribe(listener: () => void): () => void;
  load(invoke: Invoke, bundleIds: readonly string[]): Promise<void>;
}

export function createAppIconStore(): AppIconStore {
  const icons = new Map<string, string | null>();
  const pending = new Set<string>();
  const listeners = new Set<() => void>();
  let version = 0;
  return {
    icons,
    version: () => version,
    subscribe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    async load(invoke, bundleIds) {
      const wanted = [...new Set(bundleIds.map(id => id.trim()))]
        .filter(id => id !== '' && id.length <= maxBundleIdLength && !icons.has(id) && !pending.has(id));
      for (let start = 0; start < wanted.length; start += appIconBatchLimit) {
        const batch = wanted.slice(start, start + appIconBatchLimit);
        batch.forEach(id => pending.add(id));
        let result: Record<string, unknown> = {};
        try {
          const reply = await invoke<{ icons?: Record<string, unknown> } | null>('app_icons', { request: { bundle_ids: batch } });
          if (reply?.icons && typeof reply.icons === 'object') result = reply.icons;
        } catch { /* An older backend or a failed lookup: placeholders stay. */ }
        for (const id of batch) {
          const png = result[id];
          icons.set(id, typeof png === 'string' && png !== '' ? png : null);
          pending.delete(id);
        }
        version += 1;
        listeners.forEach(listener => listener());
      }
    },
  };
}

export const appIcons = createAppIconStore();
