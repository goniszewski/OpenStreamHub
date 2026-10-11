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

// `pastLabels[i]` describes the change that left `past[i]` behind, and `futureLabels` the change each redo would redo
// (the last entry is the next one), so the lists the user sees read like the steps they undo and redo.
export interface History<T> { past: T[]; future: T[]; last: EditMeta | null; pastLabels: string[]; futureLabels: string[] }

export const noLabel = 'Edited the profile';

export function emptyHistory<T>(): History<T> { return { past: [], future: [], last: null, pastLabels: [], futureLabels: [] }; }

// Consecutive edits of the same field within the window form one undo step,
// so typing in a text or number field is not one step per keystroke.
export function shouldCoalesce(previous: EditMeta | null, next: EditMeta, windowMs = coalesceWindowMs): boolean {
  if (!previous || previous.key === undefined || next.key === undefined || previous.key !== next.key) return false;
  const elapsed = next.at - previous.at;
  return elapsed >= 0 && elapsed <= windowMs;
}

const same = (a: unknown, b: unknown) => stableStringify(a) === stableStringify(b);

// Records the change `before` -> `after`. A new edit always clears the redo stack. `describe` names the change for
// the history list; a burst of coalesced edits is described from the state before the burst.
export function recordEdit<T>(history: History<T>, before: T, after: T, meta: EditMeta, limit = historyLimit, describe?: (before: T, after: T) => string): History<T> {
  if (same(before, after)) return history;
  if (shouldCoalesce(history.last, meta) && history.past.length > 0) {
    // The step already holds the state from before the burst. If the burst ended
    // back at that state, the step would be a no-op, so it is dropped. The burst
    // ends with it (last: null): otherwise the next keystroke would coalesce into
    // the unrelated step below and undo would revert that earlier change too.
    if (same(history.past[history.past.length - 1], after)) return { past: history.past.slice(0, -1), future: [], last: null, pastLabels: history.pastLabels.slice(0, -1), futureLabels: [] };
    const burstStart = history.past[history.past.length - 1];
    return { past: history.past, future: [], last: meta, pastLabels: [...history.pastLabels.slice(0, -1), describe?.(burstStart, after) ?? noLabel], futureLabels: [] };
  }
  return { past: [...history.past, before].slice(-limit), future: [], last: meta, pastLabels: [...history.pastLabels, describe?.(before, after) ?? noLabel].slice(-limit), futureLabels: [] };
}

export function undoStep<T>(history: History<T>, current: T): { history: History<T>; value: T } | null {
  if (!history.past.length) return null;
  const value = history.past[history.past.length - 1];
  const label = history.pastLabels[history.pastLabels.length - 1] ?? noLabel;
  return { value, history: { past: history.past.slice(0, -1), future: [...history.future, current], last: null, pastLabels: history.pastLabels.slice(0, -1), futureLabels: [...history.futureLabels, label] } };
}

export function redoStep<T>(history: History<T>, current: T, limit = historyLimit): { history: History<T>; value: T } | null {
  if (!history.future.length) return null;
  const value = history.future[history.future.length - 1];
  const label = history.futureLabels[history.futureLabels.length - 1] ?? noLabel;
  return { value, history: { past: [...history.past, current].slice(-limit), future: history.future.slice(0, -1), last: null, pastLabels: [...history.pastLabels, label].slice(-limit), futureLabels: history.futureLabels.slice(0, -1) } };
}

// The most recent labels first, as the list shows them: what the next undo (or redo) steps would reverse (or repeat).
export const recentUndoLabels = (history: History<unknown>, count = 10) => history.pastLabels.slice(-count).reverse();
export const recentRedoLabels = (history: History<unknown>, count = 10) => history.futureLabels.slice(-count).reverse();
