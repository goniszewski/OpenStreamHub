import type { RunMode } from './contracts';

// The editing hold behind "Switch to Test mode while editing": while the user
// is editing a binding, the run mode is held to Test (a press on any module is
// then logged, never run), and when editing ends the user's own mode is
// restored. The hold works on the stored mode itself — the session manager
// reads nothing else — so beginning it saves Test and ending it saves Live.
// A mode switch the user makes during a hold simply clears the hold, so their
// choice stands and nothing is restored underneath them.

// What a hold step decides: the new held flag, and the mode to save now, if
// any. The caller keeps live_offered as it is for both saves: holding and
// restoring never answer the one-time what-will-run confirmation.
export interface EditingStep { held: boolean; save: RunMode | null }

// Editing, as the hold sees it: unsaved changes anywhere in the open profile,
// an open action picker, or the bindings drawer. It ends on save, on undoing
// back to the saved version, or on closing the editor (the drawer or picker).
export interface EditingSignals { unsavedChanges: boolean; pickerOpen: boolean; drawerOpen: boolean }
export const isEditing = (signals: EditingSignals): boolean =>
  signals.unsavedChanges || signals.pickerOpen || signals.drawerOpen;

// What editing starting does with the hold: Test is saved over the user's
// Live, or nothing happens when the hold is already on, the setting is off,
// or the mode is not Live to begin with (Test needs no hold).
export function beginEditingHold(held: boolean, runMode: RunMode, whileEditing: boolean): EditingStep {
  if (held || !whileEditing || runMode !== 'live') return { held, save: null };
  return { held: true, save: 'test' };
}

// What editing ending (or the setting being turned off) does with an active
// hold: the mode goes back to Live. Without a hold, nothing.
export function endEditingHold(held: boolean): EditingStep {
  return held ? { held: false, save: 'live' } : { held: false, save: null };
}

// True when returning to Live at edit end must ask first: some dry run that
// would start executing was not executing when the hold began (one started
// during the hold, whose profile was never confirmed for Live), so the same
// dialog as a manual switch to Live names it before anything runs. A null
// snapshot is a hold restored from storage — the app quit or crashed mid-edit
// and this session never saw anything execute — so the user's own stored Live
// goes back in silence.
export function restoreNeedsConfirm(wereExecuting: ReadonlySet<string> | null, dryNow: readonly { serviceId: string }[]): boolean {
  return wereExecuting !== null && dryNow.some(item => !wereExecuting.has(item.serviceId));
}
