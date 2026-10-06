// Bounded undo/redo history for draft edits. Pure and immutable so the rules
// can be tested without a session.
//
// Lifetime (decided by ProfileSession): the history belongs to one draft. It is
// cleared when a different profile replaces the draft (load, switch, new,
// import, duplicate, restore) and after a successful save, because the saved
// file is then the new baseline. Undo and redo never touch the saved profile.
import { stableStringify } from './draft-model';

export const historyLimit = 100;
export const coalesceWindowMs = 600;

// `key` identifies the field being edited. Callers that pass no key never coalesce.
export interface EditMeta { key?: string; at: number }

export interface History<T> { past: T[]; future: T[]; last: EditMeta | null }

export function emptyHistory<T>(): History<T> { return { past: [], future: [], last: null }; }

// Consecutive edits of the same field within the window form one undo step,
// so typing in a text or number field is not one step per keystroke.
export function shouldCoalesce(previous: EditMeta | null, next: EditMeta, windowMs = coalesceWindowMs): boolean {
  if (!previous || previous.key === undefined || next.key === undefined || previous.key !== next.key) return false;
  const elapsed = next.at - previous.at;
  return elapsed >= 0 && elapsed <= windowMs;
}

const same = (a: unknown, b: unknown) => stableStringify(a) === stableStringify(b);

// Records the change `before` -> `after`. A new edit always clears the redo stack.
export function recordEdit<T>(history: History<T>, before: T, after: T, meta: EditMeta, limit = historyLimit): History<T> {
  if (same(before, after)) return history;
  if (shouldCoalesce(history.last, meta) && history.past.length > 0) {
    // The step already holds the state from before the burst. If the burst ended
    // back at that state, the step would be a no-op, so it is dropped. The burst
    // ends with it (last: null): otherwise the next keystroke would coalesce into
    // the unrelated step below and undo would revert that earlier change too.
    if (same(history.past[history.past.length - 1], after)) return { past: history.past.slice(0, -1), future: [], last: null };
    return { past: history.past, future: [], last: meta };
  }
  return { past: [...history.past, before].slice(-limit), future: [], last: meta };
}

export function undoStep<T>(history: History<T>, current: T): { history: History<T>; value: T } | null {
  if (!history.past.length) return null;
  const value = history.past[history.past.length - 1];
  return { value, history: { past: history.past.slice(0, -1), future: [...history.future, current], last: null } };
}

export function redoStep<T>(history: History<T>, current: T, limit = historyLimit): { history: History<T>; value: T } | null {
  if (!history.future.length) return null;
  const value = history.future[history.future.length - 1];
  return { value, history: { past: [...history.past, current].slice(-limit), future: history.future.slice(0, -1), last: null } };
}
