import type { InstallReview } from './plugins-model';

// What Install… and Remove… are doing, kept outside Settings > Plugins: the folder picker does not block the app, so
// the user can leave the page while it is open or while an install or remove runs. The picked folder's review, the
// running state and the outcome then wait here and show when the panel is open again, and Install… stays disabled
// meanwhile, so a second picker cannot open.
export interface PluginManageState {
  // The folder picker is open, or the picked folder is being reviewed.
  choosing: boolean;
  // The review of the picked folder, shown in the install dialog until it is installed or cancelled.
  install: InstallReview | null;
  // install_plugin is running.
  installing: boolean;
  // The folder name of the plugin remove_plugin is removing.
  removing: string | null;
  // The outcome of the last install or remove that failed (in full or in part), as shown in the panel.
  error: string | null;
}

export interface PluginManageStore {
  get(): PluginManageState;
  set(patch: Partial<PluginManageState>): void;
  subscribe(listener: () => void): () => void;
}

export function createPluginManageStore(): PluginManageStore {
  let state: PluginManageState = { choosing: false, install: null, installing: false, removing: null, error: null };
  const listeners = new Set<() => void>();
  return {
    get: () => state,
    set(patch) {
      state = { ...state, ...patch };
      listeners.forEach(listener => listener());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}
