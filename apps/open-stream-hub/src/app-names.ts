// Names of the apps the picker has seen (running apps, the Finder chooser, a looked-up bundle ID), by lowercase bundle
// ID, so an "Open <app>" label can say the app's name instead of its bundle ID. Unknown IDs fall back to the ID itself.
const names = new Map<string, string>();

export function rememberAppName(bundleId: string, name: string | null | undefined) {
  const id = bundleId.trim().toLowerCase();
  const text = name?.trim();
  if (id !== '' && text) names.set(id, text);
}

export const appDisplayName = (bundleId: string): string => names.get(bundleId.trim().toLowerCase()) ?? bundleId;
