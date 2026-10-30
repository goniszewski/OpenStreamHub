import type { StreamlabsCatalogNames, StreamlabsConnectionView, ObsNameList } from './contracts';
import { catalogNames, CatalogStore, isNameList, normalized, useCatalog } from './obs-catalog';
import type { CatalogSpec, CatalogView } from './obs-catalog';
import type { Invoke } from './profile-session';
import { isStreamlabsConnectionView } from './streamlabs-settings';

// What Streamlabs Desktop currently has, for the Streamlabs editor's name fields (scenes, scene collections, the items
// of one scene, audio sources), built on the OBS catalog store (obs-catalog.ts). The backend asks Streamlabs over one
// short loopback connection per reply (streamlabs_catalog, streamlabs_scene_sources in src-tauri/src/lib.rs, with the
// saved endpoint and the Keychain token, which never reaches this side) and refuses without connecting while
// Streamlabs is off or no token is set. Typing always works; the lists only offer choices and warn about names
// Streamlabs does not list.

// The items of one scene, fetched when the source field's choices open.
export type StreamlabsNameKind = 'sceneSources';

export function isStreamlabsCatalogNames(value: unknown): value is StreamlabsCatalogNames {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  const lists = ['scenes', 'scene_collections', 'audio_sources'];
  return Object.keys(record).length === lists.length + 1 && lists.every(list => isNameList(record[list]))
    && (record.active_collection === null || typeof record.active_collection === 'string');
}

const sortedCatalog = (names: StreamlabsCatalogNames): StreamlabsCatalogNames => ({
  scenes: normalized(names.scenes),
  scene_collections: normalized(names.scene_collections),
  active_collection: names.active_collection,
  audio_sources: normalized(names.audio_sources),
});

const streamlabsSpec: CatalogSpec<StreamlabsCatalogNames, StreamlabsNameKind, StreamlabsConnectionView> = {
  catalogCommand: 'streamlabs_catalog',
  statusCommand: 'streamlabs_status',
  nameCommands: { sceneSources: { command: 'streamlabs_scene_sources', argument: 'scene' } },
  isNames: isStreamlabsCatalogNames,
  sorted: sortedCatalog,
  isConnection: isStreamlabsConnectionView,
  connected: { state: 'connected', message: 'Connected to Streamlabs Desktop.' },
  unreadable: 'The Streamlabs listing reply was not understood.',
};

export type StreamlabsCatalogView = CatalogView<StreamlabsCatalogNames, StreamlabsNameKind, StreamlabsConnectionView>;

export class StreamlabsCatalogStore extends CatalogStore<StreamlabsCatalogNames, StreamlabsNameKind, StreamlabsConnectionView> {
  constructor(invoke: Invoke, desktop: boolean) {
    super(streamlabsSpec, invoke, desktop);
  }
}

export const useStreamlabsCatalog: (store: StreamlabsCatalogStore | undefined) => StreamlabsCatalogView | undefined = useCatalog;

// The scenes a scene switch can offer. Streamlabs lists only the active collection's scenes, so a switch that names
// another (or an unknown active) collection gets a plain field rather than a warning about every scene.
export function streamlabsSceneNames(catalog: StreamlabsCatalogView | undefined, collection: string): ObsNameList | null {
  if (collection && collection !== catalog?.names?.active_collection) return null;
  return catalogNames(catalog, catalog?.names?.scenes);
}
