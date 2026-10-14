import { useSyncExternalStore } from 'react';
import type { ProfileSummary } from './contracts';
import { starters } from './starter-model';
import type { StarterTemplate } from './starter-model';

// The built-in profiles are the starter templates the app ships. Each one is offered in the profile pickers (the
// header's and the sidebar's) until the user hides it in Settings; choosing one that has not been created yet saves it
// once as a normal profile and opens it. Which ones are hidden stays in the webview, like the other display preferences.
export const hiddenBuiltInsStorageKey = 'open-stream-hub.hidden-built-in-profiles';
const prefix = 'builtin:';

// What a picker lists: a saved profile, or a built-in one that is not saved yet (`value` is then `builtin:<id>`).
export interface ProfileChoice { value: string; name: string; builtIn: boolean }

export const builtInValue = (template: StarterTemplate) => `${prefix}${template.id}`;
export const builtInOf = (value: string, templates: StarterTemplate[] = starters) => value.startsWith(prefix) ? templates.find(template => builtInValue(template) === value) : undefined;

// The saved profiles in the order they come, then the built-in ones that are shown and not saved yet. A hidden built-in
// is left out even when saved, except a profile in `keep` (the open one, or one that runs), which the pickers must
// still be able to show as their value.
export function profileChoices(saved: ProfileSummary[], hidden: ReadonlySet<string>, keep: ReadonlySet<string> = new Set(), templates: StarterTemplate[] = starters): ProfileChoice[] {
  const byName = new Map(templates.map(template => [template.name, template]));
  const choices: ProfileChoice[] = saved
    .filter(item => { const template = byName.get(item.name); return !template || !hidden.has(template.id) || keep.has(item.id); })
    .map(item => ({ value: item.id, name: item.name, builtIn: false }));
  for (const template of templates) {
    if (hidden.has(template.id) || saved.some(item => item.name === template.name)) continue;
    choices.push({ value: builtInValue(template), name: template.name, builtIn: true });
  }
  return choices;
}

const parse = (text: string | null): ReadonlySet<string> => {
  try {
    const value: unknown = text === null ? [] : JSON.parse(text);
    return new Set(Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []);
  } catch { return new Set(); }
};

export function createHiddenBuiltInsStore(storage: Pick<Storage, 'getItem' | 'setItem'>) {
  let current: ReadonlySet<string> | undefined;
  const listeners = new Set<() => void>();
  const getSnapshot = () => current ??= (() => { try { return parse(storage.getItem(hiddenBuiltInsStorageKey)); } catch { return new Set<string>(); } })();
  return {
    getSnapshot,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    setShown(id: string, shown: boolean) {
      const next = new Set(getSnapshot());
      if (shown) next.delete(id); else next.add(id);
      current = next;
      try { storage.setItem(hiddenBuiltInsStorageKey, JSON.stringify([...next])); } catch { /* a convenience; keep working without it */ }
      listeners.forEach(listener => listener());
    },
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };
export const hiddenBuiltInsStore = createHiddenBuiltInsStore(webStorage);
export const useHiddenBuiltIns = (): ReadonlySet<string> => useSyncExternalStore(hiddenBuiltInsStore.subscribe, hiddenBuiltInsStore.getSnapshot, hiddenBuiltInsStore.getSnapshot);
