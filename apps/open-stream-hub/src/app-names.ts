// Names of the apps the picker has seen (running apps, the Finder chooser, a looked-up bundle ID), by lowercase bundle
// ID, so an "Open <app>" label can say the app's name instead of its bundle ID. Unknown IDs fall back to the ID itself.
const names = new Map<string, string>();
const listeners = new Set<() => void>();
let version = 0;

// For useSyncExternalStore: a component that shows app labels re-renders when a new name is learned.
export const subscribeAppNames = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
export const appNamesVersion = () => version;

export function rememberAppName(bundleId: string, name: string | null | undefined) {
  const id = bundleId.trim().toLowerCase();
  const text = name?.trim();
  if (id === '' || !text || names.get(id) === text) return;
  names.set(id, text);
  version += 1;
  listeners.forEach(listener => listener());
}

export const appDisplayName = (bundleId: string): string => names.get(bundleId.trim().toLowerCase()) ?? bundleId;
