import { describe, expect, it } from 'vitest';
import { coalesceWindowMs, emptyHistory, historyLimit, recordEdit, redoStep, shouldCoalesce, undoStep } from './edit-history';
import type { History } from './edit-history';

describe('shouldCoalesce', () => {
  it('needs the same key within the window', () => {
    expect(shouldCoalesce({ key: 'name', at: 0 }, { key: 'name', at: 100 })).toBe(true);
    expect(shouldCoalesce({ key: 'name', at: 0 }, { key: 'name', at: coalesceWindowMs })).toBe(true);
    expect(shouldCoalesce({ key: 'name', at: 0 }, { key: 'name', at: coalesceWindowMs + 1 })).toBe(false);
    expect(shouldCoalesce({ key: 'name', at: 0 }, { key: 'other', at: 10 })).toBe(false);
    expect(shouldCoalesce({ key: 'name', at: 100 }, { key: 'name', at: 50 })).toBe(false);
  });

  it('never coalesces without a key or a previous edit', () => {
    expect(shouldCoalesce(null, { key: 'name', at: 0 })).toBe(false);
    expect(shouldCoalesce({ at: 0 }, { at: 10 })).toBe(false);
    expect(shouldCoalesce({ key: 'name', at: 0 }, { at: 10 })).toBe(false);
    expect(shouldCoalesce({ at: 0 }, { key: 'name', at: 10 })).toBe(false);
  });
});

describe('history', () => {
  it('records changes, ignores no-ops and key-order differences, and clears redo', () => {
    let h: History<unknown> = emptyHistory();
    h = recordEdit(h, { a: 1, b: 2 }, { b: 2, a: 1 }, { at: 0 });
    expect(h.past).toEqual([]);
    h = recordEdit(h, { a: 1 }, { a: 2 }, { at: 1 });
    expect(h.past).toEqual([{ a: 1 }]);
    const undone = undoStep(h, { a: 2 })!;
    expect(undone.value).toEqual({ a: 1 });
    expect(undone.history.future).toEqual([{ a: 2 }]);
    const next = recordEdit(undone.history, { a: 1 }, { a: 3 }, { at: 2 });
    expect(next.future).toEqual([]);
    expect(next.past).toEqual([{ a: 1 }]);
  });

  it('coalesces a burst of keyed edits into one step and keeps unkeyed edits separate', () => {
    let h: History<string> = emptyHistory();
    let value = '';
    let at = 0;
    for (const letter of 'hello') { const before = value; value += letter; at += 100; h = recordEdit(h, before, value, { key: 'name', at }); }
    expect(h.past).toEqual(['']);
    at += 2000;
    h = recordEdit(h, value, `${value}!`, { key: 'name', at });
    expect(h.past).toEqual(['', 'hello']);
    h = recordEdit(h, 'hello!', 'hello!!', { at: at + 1 });
    h = recordEdit(h, 'hello!!', 'hello!!!', { at: at + 2 });
    expect(h.past).toHaveLength(4);
  });

  it('drops a coalesced step that ends where it began', () => {
    let h: History<string> = emptyHistory();
    h = recordEdit(h, 'a', 'ab', { key: 'f', at: 0 });
    h = recordEdit(h, 'ab', 'a', { key: 'f', at: 100 });
    expect(h.past).toEqual([]);
    h = recordEdit(h, 'a', 'ax', { key: 'f', at: 200 });
    expect(h.past).toEqual(['a']);
  });

  it('is bounded to the newest entries', () => {
    let h: History<number> = emptyHistory();
    for (let value = 0; value < historyLimit + 20; value += 1) h = recordEdit(h, value, value + 1, { at: value });
    expect(h.past).toHaveLength(historyLimit);
    expect(h.past[0]).toBe(20);
    expect(h.past.at(-1)).toBe(historyLimit + 19);
    const full: History<number> = { past: Array.from({ length: historyLimit }, (_, index) => index), future: [999], last: null };
    expect(redoStep(full, -1)!.history.past).toHaveLength(historyLimit);
  });

  it('round trips undo and redo and does nothing when empty', () => {
    expect(undoStep(emptyHistory<string>(), 'x')).toBeNull();
    expect(redoStep(emptyHistory<string>(), 'x')).toBeNull();
    let h: History<string> = recordEdit(emptyHistory(), 'a', 'b', { at: 0 });
    h = recordEdit(h, 'b', 'c', { at: 1 });
    const u1 = undoStep(h, 'c')!;
    expect(u1.value).toBe('b');
    const u2 = undoStep(u1.history, u1.value)!;
    expect(u2.value).toBe('a');
    const r1 = redoStep(u2.history, u2.value)!;
    expect(r1.value).toBe('b');
    const r2 = redoStep(r1.history, r1.value)!;
    expect(r2.value).toBe('c');
    expect(redoStep(r2.history, r2.value)).toBeNull();
    // After undo the next keyed edit must not merge into the pre-undo burst.
    expect(u1.history.last).toBeNull();
  });
});
