import { useEffect, useState } from 'react';
import type { LightingDevice } from './lighting-model';
import type { Invoke } from './profile-session';

// A reply is trusted only if it has the shape of a base entry.
export function parseBaseModules(value: unknown): LightingDevice[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item): LightingDevice[] => {
    const entry = item as { path_hex?: unknown; serial_number?: unknown } | null;
    if (!entry || typeof entry.path_hex !== 'string') return [];
    return [{ path_hex: entry.path_hex, module: 'm0064', serial_number: typeof entry.serial_number === 'string' ? entry.serial_number : null }];
  });
}

// The connected bases, which the input feed never lists (a base has no input).
// The list is read once and again whenever the feed's inventory changes, since
// a base joins and leaves the same USB inventory as the modules on it. A failed
// read leaves the last list in place.
export function useBaseModules(invoke: Invoke | undefined, inventoryKey: string): LightingDevice[] {
  const [bases, setBases] = useState<LightingDevice[]>([]);
  useEffect(() => {
    if (!invoke) return;
    let current = true;
    invoke('list_base_modules', {}).then(
      reply => { if (current) setBases(parseBaseModules(reply)); },
      () => {},
    );
    return () => { current = false; };
  }, [invoke, inventoryKey]);
  return invoke ? bases : [];
}
