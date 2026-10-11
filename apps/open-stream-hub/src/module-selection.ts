import type { LightingDevice } from './lighting-model';

export const selectionStorageKey = 'open-stream-hub.selected-module';

export interface SelectionState {
  // The picked module's path, or null when nothing was picked yet.
  path: string | null;
  // Modules a page wants flagged on the strip (Lighting's Whole kit marks the
  // modules it will reach). Pages set it while mounted and clear it on leaving.
  marked: string[];
}

export interface ModuleSelection {
  subscribe: (listener: () => void) => () => void;
  getState: () => SelectionState;
  select: (path: string) => void;
  mark: (paths: string[]) => void;
}

// The picked module, kept across pages and restarts. Storage may be missing or
// refuse access; the pick then lasts for the session only.
export function createModuleSelection(storage: Pick<Storage, 'getItem' | 'setItem'> | null): ModuleSelection {
  let state: SelectionState = { path: readPath(storage), marked: [] };
  const listeners = new Set<() => void>();
  const set = (next: SelectionState) => {
    state = next;
    listeners.forEach(listener => listener());
  };
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    getState: () => state,
    select(path) {
      if (path === state.path) return;
      try { storage?.setItem(selectionStorageKey, path); } catch { /* the pick stays for this session only */ }
      set({ ...state, path });
    },
    mark(paths) {
      if (paths.length === state.marked.length && paths.every((path, index) => path === state.marked[index])) return;
      set({ ...state, marked: paths });
    },
  };
}

function readPath(storage: Pick<Storage, 'getItem'> | null): string | null {
  try { return storage?.getItem(selectionStorageKey) || null; } catch { return null; }
}

// The picked module if it is still connected; otherwise the first dial, then
// the first module, so a replug never leaves the strip without a pick.
export function resolveSelected(devices: LightingDevice[], path: string | null): LightingDevice | undefined {
  return devices.find(device => device.path_hex === path)
    ?? devices.find(device => device.module === 'm131e') ?? devices[0];
}
