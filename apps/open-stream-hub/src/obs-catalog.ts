import { useEffect, useSyncExternalStore } from 'react';
import { maxObsSceneBytes } from './action-model';
import type { ObsCatalogNames, ObsNameList } from './contracts';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

// What OBS currently has, for the OBS editor's name fields (scenes, sources, filters, scene
// collections, profiles). The backend asks OBS over one short loopback connection per reply
// (obs_catalog, obs_source_filters, obs_scene_sources in src-tauri/src/lib.rs, using the saved
// endpoint and the in-memory password), so a listing is exactly as current as the Refresh press.
// When OBS is off or unreachable, the fields stay plain text inputs — typing always works, the
// lists only offer choices and warn about names OBS does not list.

// Mirrors MAX_LISTED_NAMES in crates/cooltek-sdk-service/src/obs.rs.
const maxListedNames = 1000;
// The backend drops listed names beyond the profile's own name limit (MAX_LISTED_NAME_BYTES there
// equals Action::MAX_OBS_NAME_BYTES), so a request for a longer name could never be answered with
// anything the editor could save.
const maxListableNameBytes = maxObsSceneBytes;

const utf8 = new TextEncoder();

// Which per-name list a field needs beyond the catalog: the filters on one source, or the items
// of one scene (what a source visibility there can target).
export type ObsNameKind = 'filters' | 'sceneSources';
const nameCommands: Record<ObsNameKind, { command: string; argument: string }> = {
  filters: { command: 'obs_source_filters', argument: 'source' },
  sceneSources: { command: 'obs_scene_sources', argument: 'scene' },
};
const listKey = (kind: ObsNameKind, name: string) => `${kind}:${name}`;

export interface ObsCatalogState {
  desktop: boolean;
  status: 'idle' | 'loading' | 'ready' | 'failed';
  names: ObsCatalogNames | null;
  error: string | null;
  // Per-name lists fetched on demand ("filters:Cam", "sceneSources:Live"), never prefetched.
  lists: Record<string, ObsNameList>;
}

export const idleObsCatalogState: ObsCatalogState = { desktop: false, status: 'idle', names: null, error: null, lists: {} };

export function isObsCatalogNames(value: unknown): value is ObsCatalogNames {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  const lists = ['scenes', 'sources', 'scene_collections', 'profiles'];
  return Object.keys(record).length === lists.length && lists.every(list => {
    const names = record[list];
    return Array.isArray(names) && names.length <= maxListedNames && names.every(name => typeof name === 'string');
  });
}

// Case-insensitive order, numbers inside names in natural order ("Cam 2" before "Cam 10").
function byName(a: string, b: string): number {
  return a.localeCompare(b, undefined, { sensitivity: 'base', numeric: true });
}

function normalized(names: string[]): string[] {
  return [...new Set(names)].sort(byName);
}

function sortedCatalog(names: ObsCatalogNames): ObsCatalogNames {
  return {
    scenes: normalized(names.scenes),
    sources: normalized(names.sources),
    scene_collections: normalized(names.scene_collections),
    profiles: normalized(names.profiles),
  };
}

// A catalog list an editor field can offer (scenes, scene collections, profiles), as an
// ObsNameList: loading while the catalog is being fetched, the names once it arrived, and `null`
// wherever there is no list (no catalog, OBS off or unreachable) so the field stays a plain input.
export function catalogNames(catalog: ObsCatalogState | undefined, names: string[] | undefined): ObsNameList | null {
  if (catalog?.status === 'loading') return { status: 'loading' };
  return names ? { status: 'ready', names } : null;
}

// What an OBS filter can attach to: every input plus every scene (OBS carries filters on scenes
// too). Null keeps the field a plain input.
export function filterTargets(catalog: ObsCatalogState | undefined): ObsNameList | null {
  if (catalog?.status === 'loading') return { status: 'loading' };
  if (!catalog?.names) return null;
  return { status: 'ready', names: normalized([...catalog.names.sources, ...catalog.names.scenes]) };
}

export interface ObsCatalogView extends ObsCatalogState {
  refresh(): void;
  // The cached per-name list, or null when none was requested yet (render never starts a fetch).
  list(kind: ObsNameKind, name: string): ObsNameList | null;
  // Starts the fetch for a per-name list (from the field's open event, not from render).
  requestNames(kind: ObsNameKind, name: string): void;
}

export class ObsCatalogStore {
  private state: ObsCatalogState;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly invoke: Invoke, desktop: boolean) {
    this.state = { ...idleObsCatalogState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<ObsCatalogState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Asks OBS for everything at once. A failure keeps the field inputs plain and the note explains
  // why; a success drops every cached per-name list, which may be as stale as the catalog was.
  readonly refresh = async (): Promise<boolean> => {
    if (!this.state.desktop || this.state.status === 'loading') return false;
    this.set({ status: 'loading', error: null });
    try {
      const names = await this.invoke<unknown>('obs_catalog');
      if (!isObsCatalogNames(names)) throw new Error('The OBS listing reply was not understood.');
      this.set({ status: 'ready', names: sortedCatalog(names), error: null, lists: {} });
      return true;
    } catch (reason) {
      this.set({ status: 'failed', error: errorMessage(reason) });
      return false;
    }
  };

  readonly list = (kind: ObsNameKind, name: string): ObsNameList | null => this.state.lists[listKey(kind, name)] ?? null;

  // Called from a field's open event, never from render. A name too long for the backend's
  // listings is ignored: the field's own validation already asks for a shorter one.
  readonly requestNames = (kind: ObsNameKind, name: string): void => {
    const key = listKey(kind, name);
    if (!this.state.desktop || this.state.lists[key] || !name.trim() || utf8.encode(name).length > maxListableNameBytes) return;
    const { command, argument } = nameCommands[kind];
    this.set({ lists: { ...this.state.lists, [key]: { status: 'loading' } } });
    void (async () => {
      try {
        const names = await this.invoke<unknown>(command, { [argument]: name });
        if (!Array.isArray(names) || names.length > maxListedNames || !names.every(entry => typeof entry === 'string')) throw new Error('The OBS listing reply was not understood.');
        this.set({ lists: { ...this.state.lists, [key]: { status: 'ready', names: normalized(names) } } });
      } catch (reason) {
        this.set({ lists: { ...this.state.lists, [key]: { status: 'failed', error: errorMessage(reason) } } });
      }
    })();
  };

  // Browser-preview QA only: the preview cannot reach Tauri, so design captures seed sample
  // listings through it. The desktop app never calls this.
  seedPreview(names: ObsCatalogNames) {
    if (this.state.desktop || !isObsCatalogNames(names)) return;
    this.set({ status: 'ready', names: sortedCatalog(names), error: null, lists: {} });
  }
}

// Browser-preview QA seeding (dev only): App points this at the mounted store, so the
// window.__oshObsCatalogSeed hook reaches the live instance across StrictMode's double mount.
let previewStore: ObsCatalogStore | null = null;
export function exposePreviewSeeding(store: ObsCatalogStore): void {
  if (!store.getState().desktop) previewStore = store;
}
export function seedPreviewCatalog(names: ObsCatalogNames): void {
  previewStore?.seedPreview(names);
}

const noopSubscribe = () => () => {};
const idleCatalog = () => idleObsCatalogState;

// The catalog for one editor. Mounting it asks OBS once per app session (the Refresh press asks
// again); without a store — the browser preview, older call sites — the fields stay plain inputs.
export function useObsCatalog(store: ObsCatalogStore | undefined): ObsCatalogView | undefined {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idleCatalog, store?.getState ?? idleCatalog);
  useEffect(() => { if (store && state.desktop && state.status === 'idle') void store.refresh(); }, [store, state.desktop, state.status]);
  if (!store) return undefined;
  return { ...state, refresh: () => { void store.refresh(); }, list: store.list, requestNames: store.requestNames };
}
