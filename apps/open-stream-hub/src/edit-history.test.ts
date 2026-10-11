import { describe, expect, it } from 'vitest';
import { coalesceWindowMs, emptyHistory, historyLimit, noLabel, recentRedoLabels, recentUndoLabels, recordEdit, redoStep, shouldCoalesce, undoStep } from './edit-history';
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

  it('ends the burst when it drops a step, so the next keystroke does not merge into the step below', () => {
    type Draft = { binding: string; name: string };
    let h: History<Draft> = emptyHistory();
    let draft: Draft = { binding: 'S0', name: '' };
    const edit = (next: Draft, meta: { key?: string; at: number }) => { h = recordEdit(h, draft, next, meta); draft = next; };
    edit({ binding: 'X', name: '' }, { at: 0 });
    edit({ binding: 'X', name: 'a' }, { key: 'name', at: 1000 });
    edit({ binding: 'X', name: '' }, { key: 'name', at: 1100 });
    expect(h.past).toEqual([{ binding: 'S0', name: '' }]);
    expect(h.last).toBeNull();
    edit({ binding: 'X', name: 'b' }, { key: 'name', at: 1200 });
    const undone = undoStep(h, draft)!;
    expect(undone.value).toEqual({ binding: 'X', name: '' });
    expect(undoStep(undone.history, undone.value)!.value).toEqual({ binding: 'S0', name: '' });
  });

  it('is bounded to the newest entries', () => {
    let h: History<number> = emptyHistory();
    for (let value = 0; value < historyLimit + 20; value += 1) h = recordEdit(h, value, value + 1, { at: value });
    expect(h.past).toHaveLength(historyLimit);
    expect(h.past[0]).toBe(20);
    expect(h.past.at(-1)).toBe(historyLimit + 19);
    const full: History<number> = { past: Array.from({ length: historyLimit }, (_, index) => index), future: [999], last: null, pastLabels: Array.from({ length: historyLimit }, (_, index) => `step ${index}`), futureLabels: ['later'] };
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

describe('history labels', () => {
  const describe_ = (before: string, after: string) => `${before}→${after}`;

  it('keeps one label per step and moves it between the undo and redo lists', () => {
    let h: History<string> = emptyHistory();
    h = recordEdit(h, 'a', 'b', { at: 0 }, undefined, describe_);
    h = recordEdit(h, 'b', 'c', { at: 1 }, undefined, describe_);
    expect(h.pastLabels).toEqual(['a→b', 'b→c']);
    expect(recentUndoLabels(h)).toEqual(['b→c', 'a→b']);
    const undone = undoStep(h, 'c')!;
    expect(undone.history.pastLabels).toEqual(['a→b']);
    expect(undone.history.futureLabels).toEqual(['b→c']);
    expect(recentRedoLabels(undone.history)).toEqual(['b→c']);
    const redone = redoStep(undone.history, undone.value)!;
    expect(redone.history.pastLabels).toEqual(['a→b', 'b→c']);
    expect(redone.history.futureLabels).toEqual([]);
  });

  it('describes a coalesced burst from the state before it, and drops the label with a burst that undid itself', () => {
    let h: History<string> = emptyHistory();
    let value = '';
    let at = 0;
    for (const letter of 'hi') { const before = value; value += letter; at += 100; h = recordEdit(h, before, value, { key: 'name', at }, undefined, describe_); }
    expect(h.past).toEqual(['']);
    expect(h.pastLabels).toEqual(['→hi']);
    h = recordEdit(h, 'hi', '', { key: 'name', at: at + 100 }, undefined, describe_);
    expect(h.past).toEqual([]);
    expect(h.pastLabels).toEqual([]);
  });

  it('a new edit clears the redo labels, the limit trims the oldest label with its state, and a missing describer uses the plain phrase', () => {
    let h: History<number> = emptyHistory();
    h = recordEdit(h, 0, 1, { at: 0 });
    expect(h.pastLabels).toEqual([noLabel]);
    h = undoStep(h, 1)!.history;
    expect(h.futureLabels).toHaveLength(1);
    h = recordEdit(h, 0, 5, { at: 1 }, undefined, (before, after) => `${before}-${after}`);
    expect(h.futureLabels).toEqual([]);
    let full: History<number> = emptyHistory();
    for (let value = 0; value < historyLimit + 5; value += 1) full = recordEdit(full, value, value + 1, { at: value }, undefined, (before) => `from ${before}`);
    expect(full.pastLabels).toHaveLength(historyLimit);
    expect(full.pastLabels[0]).toBe('from 5');
    expect(full.pastLabels.at(-1)).toBe(`from ${historyLimit + 4}`);
  });

  it('lists at most ten labels, most recent first', () => {
    let h: History<number> = emptyHistory();
    for (let value = 0; value < 15; value += 1) h = recordEdit(h, value, value + 1, { at: value }, undefined, (before) => `step ${before}`);
    expect(recentUndoLabels(h)).toHaveLength(10);
    expect(recentUndoLabels(h)[0]).toBe('step 14');
  });
});
