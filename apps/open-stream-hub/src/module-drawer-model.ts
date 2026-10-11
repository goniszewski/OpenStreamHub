import { moduleAcceptsDisplayArtwork, moduleArtworkCaption } from './artwork-model';
import type { ModuleId } from './ui-model';
import { lightingCapability, lightingKind } from './lighting-model';
import type { LightingDevice, LightingModule } from './lighting-model';
import { resolveSelected } from './module-selection';
import { modules } from './ui-model';

// The pages the module drawer sits above, each listing the modules it can act on.
export type DrawerPage = 'workspace' | 'artwork' | 'lighting';

export interface DrawerTile {
  key: string;
  module: LightingModule;
  // The unit's path, or null for a module kind that is not connected (Workspace edits its bindings anyway).
  path: string | null;
  serial: string | null;
  caption: string;
  picked: boolean;
  // Flagged by a page (Lighting's Whole kit marks the modules it reaches).
  marked: boolean;
  // The current page cannot act on it; the tile is dimmed but can still be picked.
  dimmed: boolean;
}

interface DrawerInput {
  page: DrawerPage;
  // Connected units, the base first, as the pages list them.
  devices: LightingDevice[];
  selectedPath: string | null;
  markedPaths: string[];
  // Workspace's own pick: a module kind, because bindings belong to a kind.
  editorKind: ModuleId;
}

// The tiles for one page. Lighting and Artwork list the connected units, one tile each, and pick the
// shared selection. Workspace lists every module kind (several units of a kind get a tile each; a
// kind with none is one "Not connected" tile) and picks by the editor's kind, so the shared pick wins
// only when it is a unit of that kind.
export function drawerTiles({ page, devices, selectedPath, markedPaths, editorKind }: DrawerInput): DrawerTile[] {
  const selected = resolveSelected(devices, selectedPath);
  if (page !== 'workspace') {
    return devices.map(device => ({
      key: device.path_hex, module: device.module, path: device.path_hex, serial: device.serial_number,
      caption: page === 'artwork' ? moduleArtworkCaption(device) : lightingCapability(device.module),
      picked: device.path_hex === selected?.path_hex,
      marked: markedPaths.includes(device.path_hex),
      dimmed: page === 'artwork' ? !moduleAcceptsDisplayArtwork(device) : lightingKind(device.module) === null,
    }));
  }
  const tiles: DrawerTile[] = [];
  for (const kind of modules) {
    const units = devices.filter(device => device.module === kind.id);
    const own = kind.id === editorKind;
    const pickedUnit = own && selected?.module === kind.id ? selected.path_hex : units[0]?.path_hex ?? null;
    if (units.length === 0) {
      tiles.push({ key: kind.id, module: kind.id, path: null, serial: null, caption: 'Not connected', picked: own, marked: false, dimmed: true });
      continue;
    }
    for (const unit of units) {
      tiles.push({ key: unit.path_hex, module: kind.id, path: unit.path_hex, serial: unit.serial_number, caption: 'Connected',
        picked: own && unit.path_hex === pickedUnit, marked: false, dimmed: false });
    }
  }
  return tiles;
}

export const drawerStorageKey = 'open-stream-hub.module-drawer';

// The drawer is open unless the user closed it; storage may be missing or refuse access.
export function readDrawerOpen(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(drawerStorageKey) !== 'closed'; } catch { return true; }
}

export function writeDrawerOpen(storage: Pick<Storage, 'setItem'> | null, open: boolean): void {
  try { storage?.setItem(drawerStorageKey, open ? 'open' : 'closed'); } catch { /* the drawer stays as it is for this session */ }
}

export interface SyncedPicks { path: string | null; kind: ModuleId }

// What to do after the shared pick or the editor's kind changed. The editor holds a module kind and the
// shared pick a unit, so they follow each other: a unit picked elsewhere opens its kind in the editor,
// and a kind opened in the editor moves the pick to a unit of that kind. `previous` is what the two
// were when last settled, so only a change since then acts (never the first render, and never twice
// for one change); the base has no bindings and never opens an editor kind.
export function reconcilePicks({ previous, pickPath, editorKind, devices }: {
  previous: SyncedPicks; pickPath: string | null; editorKind: ModuleId; devices: LightingDevice[];
}): { act: 'none'; next: SyncedPicks } | { act: 'editor'; kind: ModuleId; next: SyncedPicks } | { act: 'selection'; path: string; next: SyncedPicks } {
  const resolved = resolveSelected(devices, pickPath);
  if (pickPath !== previous.path && resolved && resolved.module !== 'm0064' && resolved.module !== editorKind) {
    return { act: 'editor', kind: resolved.module, next: { path: pickPath, kind: resolved.module } };
  }
  if (editorKind !== previous.kind && resolved?.module !== editorKind) {
    const unit = devices.find(device => device.module === editorKind);
    if (unit) return { act: 'selection', path: unit.path_hex, next: { path: unit.path_hex, kind: editorKind } };
  }
  return { act: 'none', next: { path: pickPath, kind: editorKind } };
}
