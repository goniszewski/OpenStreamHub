import { describe, expect, it } from 'vitest';
import { faderMemoryKey, readFaderMemory, writeFaderMemory } from './fader-memory';

const store = (initial?: string) => { const data = new Map<string, string>(initial === undefined ? [] : [[faderMemoryKey, initial]]); return { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); } }; };

describe('fader memory', () => {
  it('returns what was written', () => {
    const storage = store();
    writeFaderMemory(storage, [0, 25, 50, 75, 100]);
    expect(readFaderMemory(storage)).toEqual([0, 25, 50, 75, 100]);
  });
  it('ignores missing, malformed and out-of-range data', () => {
    expect(readFaderMemory(store())).toBeNull();
    expect(readFaderMemory(store('nope'))).toBeNull();
    expect(readFaderMemory(store('[1,2,3]'))).toBeNull();
    expect(readFaderMemory(store('[0,0,0,0,101]'))).toBeNull();
    expect(readFaderMemory(null)).toBeNull();
  });
  it('survives a storage that throws', () => {
    expect(() => writeFaderMemory({ setItem: () => { throw new Error('denied'); } }, [1, 2, 3, 4, 5])).not.toThrow();
    expect(readFaderMemory({ getItem: () => { throw new Error('denied'); } })).toBeNull();
  });
});
