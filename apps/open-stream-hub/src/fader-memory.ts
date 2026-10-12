// The last fader positions the module reported, kept so the base layout can show them again before the faders move.
export const faderMemoryKey = 'open-stream-hub.fader-positions';

export function readFaderMemory(storage: Pick<Storage, 'getItem'> | null): number[] | null {
  try {
    const parsed: unknown = JSON.parse(storage?.getItem(faderMemoryKey) ?? 'null');
    return Array.isArray(parsed) && parsed.length === 5 && parsed.every(value => typeof value === 'number' && value >= 0 && value <= 100) ? parsed : null;
  } catch { return null; }
}

export function writeFaderMemory(storage: Pick<Storage, 'setItem'> | null, positions: readonly number[]): void {
  try { storage?.setItem(faderMemoryKey, JSON.stringify(positions)); } catch { /* the positions are simply not remembered */ }
}
