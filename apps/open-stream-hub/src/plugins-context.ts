import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import type { PluginListing } from './plugins-model';

// The installed plugins, read once when the editor opens and again on refresh. Outside the desktop app (or before the
// first read) the listing is null and plugin actions are edited without manifest checks.
export interface PluginsView { listing: PluginListing | null; error: string | null; refresh(): void }

export const PluginsContext = createContext<PluginsView>({ listing: null, error: null, refresh: () => {} });
export const usePlugins = (): PluginsView => useContext(PluginsContext);

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function usePluginListing(invoke?: Invoke): PluginsView {
  const [state, setState] = useState<{ listing: PluginListing | null; error: string | null }>({ listing: null, error: null });
  const [generation, setGeneration] = useState(0);
  // Callers often pass a fresh closure each render; only whether one exists starts a read.
  const latest = useRef(invoke);
  latest.current = invoke;
  const available = !!invoke;
  useEffect(() => {
    const read = latest.current;
    if (!read) return;
    let current = true;
    read<PluginListing>('list_plugins')
      .then(listing => { if (current) setState({ listing, error: null }); })
      .catch((reason: unknown) => { if (current) setState({ listing: null, error: String(reason) }); });
    return () => { current = false; };
  }, [available, generation]);
  const refresh = useCallback(() => setGeneration(value => value + 1), []);
  return { ...state, refresh };
}
