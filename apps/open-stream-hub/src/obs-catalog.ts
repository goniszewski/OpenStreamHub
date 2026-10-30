import { useEffect, useSyncExternalStore } from 'react';
import { maxObsSceneBytes } from './action-model';
import type { ObsCatalogNames, ObsConnectionView, ObsNameList } from './contracts';
import { isObsConnectionView } from './obs-settings';
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
const listKey = (kind: string, name: string) => `${kind}:${name}`;

// A service's listings for the editor's name fields, shared by OBS and Streamlabs
// (streamlabs-catalog.ts): the catalog reply, the per-name lists and their readiness.
export interface CatalogState<Names, Connection> {
  desktop: boolean;
  status: 'idle' | 'loading' | 'ready' | 'failed';
  names: Names | null;
  error: string | null;
  // Per-name lists fetched on demand ("filters:Cam", "sceneSources:Live"), never prefetched.
  lists: Record<string, ObsNameList>;
  // Why the service can or cannot be used, from the last refresh: connected after a listing, else the backend's verdict.
  connection: Connection | null;
}
export type ObsCatalogState = CatalogState<ObsCatalogNames, ObsConnectionView>;

export const idleCatalogState: CatalogState<never, never> = { desktop: false, status: 'idle', names: null, error: null, lists: {}, connection: null };
export const idleObsCatalogState: ObsCatalogState = idleCatalogState;

export function isObsCatalogNames(value: unknown): value is ObsCatalogNames {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  const lists = ['scenes', 'sources', 'scene_collections', 'profiles'];
  return Object.keys(record).length === lists.length && lists.every(list => isNameList(record[list]));
}

// A listing reply's list: at most the backend's bound, strings only.
export function isNameList(value: unknown): value is string[] {
  return Array.isArray(value) && value.length <= maxListedNames && value.every(name => typeof name === 'string');
}

// Case-insensitive order, numbers inside names in natural order ("Cam 2" before "Cam 10").
function byName(a: string, b: string): number {
  return a.localeCompare(b, undefined, { sensitivity: 'base', numeric: true });
}

export function normalized(names: string[]): string[] {
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
// wherever there is no list (no catalog, the service off or unreachable) so the field stays a plain input.
export function catalogNames(catalog: CatalogState<unknown, unknown> | undefined, names: string[] | undefined): ObsNameList | null {
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

export interface CatalogView<Names, Kind extends string, Connection> extends CatalogState<Names, Connection> {
  refresh(): void;
  // The cached per-name list, or null when none was requested yet (render never starts a fetch).
  list(kind: Kind, name: string): ObsNameList | null;
  // Starts the fetch for a per-name list (from the field's open event, not from render).
  requestNames(kind: Kind, name: string): void;
}
export type ObsCatalogView = CatalogView<ObsCatalogNames, ObsNameKind, ObsConnectionView>;

// What tells one service's listings apart: its commands, reply checks and wording.
export interface CatalogSpec<Names, Kind extends string, Connection> {
  catalogCommand: string;
  statusCommand: string;
  nameCommands: Record<Kind, { command: string; argument: string }>;
  isNames(value: unknown): value is Names;
  sorted(names: Names): Names;
  isConnection(value: unknown): value is Connection;
  connected: Connection;
  unreadable: string;
}

export class CatalogStore<Names, Kind extends string, Connection> {
  private state: CatalogState<Names, Connection>;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly spec: CatalogSpec<Names, Kind, Connection>, private readonly invoke: Invoke, desktop: boolean) {
    this.state = { ...idleCatalogState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  protected set(patch: Partial<CatalogState<Names, Connection>>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Asks the service for everything at once. A failure keeps the field inputs plain and the note
  // explains why; a success drops every cached per-name list, which may be as stale as the catalog was.
  readonly refresh = async (): Promise<boolean> => {
    if (!this.state.desktop || this.state.status === 'loading') return false;
    this.set({ status: 'loading', error: null, connection: null });
    try {
      const names = await this.invoke<unknown>(this.spec.catalogCommand);
      if (!this.spec.isNames(names)) throw new Error(this.spec.unreadable);
      this.set({ status: 'ready', names: this.spec.sorted(names), error: null, lists: {}, connection: this.spec.connected });
      return true;
    } catch (reason) {
      this.set({ status: 'failed', error: errorMessage(reason) });
      await this.diagnose();
      return false;
    }
  };

  // After a failed listing, asks the backend which of the known states this is. Best effort: the error text stays.
  private async diagnose(): Promise<void> {
    try {
      const connection = await this.invoke<unknown>(this.spec.statusCommand);
      if (this.spec.isConnection(connection)) this.set({ connection });
    } catch { /* the listing error already says it failed */ }
  }

  readonly list = (kind: Kind, name: string): ObsNameList | null => this.state.lists[listKey(kind, name)] ?? null;

  // Called from a field's open event, never from render. A name too long for the backend's
  // listings is ignored: the field's own validation already asks for a shorter one.
  readonly requestNames = (kind: Kind, name: string): void => {
    const key = listKey(kind, name);
    if (!this.state.desktop || this.state.lists[key] || !name.trim() || utf8.encode(name).length > maxListableNameBytes) return;
    const { command, argument } = this.spec.nameCommands[kind];
    this.set({ lists: { ...this.state.lists, [key]: { status: 'loading' } } });
    void (async () => {
      try {
        const names = await this.invoke<unknown>(command, { [argument]: name });
        if (!isNameList(names)) throw new Error(this.spec.unreadable);
        this.set({ lists: { ...this.state.lists, [key]: { status: 'ready', names: normalized(names) } } });
      } catch (reason) {
        this.set({ lists: { ...this.state.lists, [key]: { status: 'failed', error: errorMessage(reason) } } });
      }
    })();
  };
}

const obsSpec: CatalogSpec<ObsCatalogNames, ObsNameKind, ObsConnectionView> = {
  catalogCommand: 'obs_catalog',
  statusCommand: 'obs_status',
  nameCommands: {
    filters: { command: 'obs_source_filters', argument: 'source' },
    sceneSources: { command: 'obs_scene_sources', argument: 'scene' },
  },
  isNames: isObsCatalogNames,
  sorted: sortedCatalog,
  isConnection: isObsConnectionView,
  connected: { state: 'connected', message: 'Connected to OBS.' },
  unreadable: 'The OBS listing reply was not understood.',
};

export class ObsCatalogStore extends CatalogStore<ObsCatalogNames, ObsNameKind, ObsConnectionView> {
  constructor(invoke: Invoke, desktop: boolean) {
    super(obsSpec, invoke, desktop);
  }

  // Browser-preview QA only: the preview cannot reach Tauri, so design captures seed sample
  // listings through it. The desktop app never calls this.
  seedPreview(names: ObsCatalogNames) {
    if (this.getState().desktop || !isObsCatalogNames(names)) return;
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
const idleCatalog = () => idleCatalogState;

// The catalog for one editor. Mounting it asks the service once per app session (the Refresh press
// asks again); without a store — the browser preview, older call sites — the fields stay plain inputs.
export function useCatalog<Names, Kind extends string, Connection>(store: CatalogStore<Names, Kind, Connection> | undefined): CatalogView<Names, Kind, Connection> | undefined {
  const state = useSyncExternalStore<CatalogState<Names, Connection>>(store?.subscribe ?? noopSubscribe, store?.getState ?? idleCatalog, store?.getState ?? idleCatalog);
  useEffect(() => { if (store && state.desktop && state.status === 'idle') void store.refresh(); }, [store, state.desktop, state.status]);
  if (!store) return undefined;
  return { ...state, refresh: () => { void store.refresh(); }, list: store.list, requestNames: store.requestNames };
}

export const useObsCatalog: (store: ObsCatalogStore | undefined) => ObsCatalogView | undefined = useCatalog;
