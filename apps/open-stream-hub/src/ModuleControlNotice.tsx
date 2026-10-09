import { Info, X } from 'lucide-react';
import type { ModuleControl } from './contracts';

// One non-blocking notice, shown once per user, that says the app drives the 15-key module and the dial (Module control
// App, the default): the session manager starts a dry run unless automatic actions are enabled in Settings.
export const moduleControlNoticeText = 'OpenStreamHub drives the 15-key module and the encoder dial whenever it is open; their built-in animation returns when you quit. Change this in Settings, under “Let OpenStreamHub drive the display”.';
// Added only while "Run actions automatically" is on.
export const autoActionsNoticeText = 'Actions run automatically; turn that off in Settings.';
export const moduleControlNoticeStorageKey = 'open-stream-hub.module-control-notice-shown';

// True until the notice was shown once. If storage cannot be read the answer is unknown, so the notice stays hidden
// rather than reappearing on every launch (like the getting-started guide).
export function shouldShowModuleControlNotice(storage: Pick<Storage, 'getItem'> | null, desktop: boolean): boolean {
  if (!desktop || !storage) return false;
  try { return storage.getItem(moduleControlNoticeStorageKey) === null; } catch { return false; }
}

// Whether the notice is on screen now: still pending, Module control known to be App, and not covered by the
// getting-started guide (a modal), so it is never recorded as shown while nobody could see it.
export function moduleControlNoticeVisible(input: { pending: boolean; guideOpen: boolean; loaded: boolean; moduleControl: ModuleControl }): boolean {
  return input.pending && !input.guideOpen && input.loaded && input.moduleControl === 'app';
}

// Called as soon as the notice is on screen, so it is shown once even when it is never dismissed.
export function markModuleControlNoticeShown(storage: Pick<Storage, 'setItem'> | null) {
  try { storage?.setItem(moduleControlNoticeStorageKey, 'shown'); } catch { /* Shown again next launch if storage is blocked. */ }
}

export function ModuleControlNotice({ autoExecute = false, onDismiss }: { autoExecute?: boolean; onDismiss(): void }) {
  return <div className="alert notice module-control-notice" role="status">
    <Info size={18} aria-hidden="true" /><span>{autoExecute ? `${moduleControlNoticeText} ${autoActionsNoticeText}` : moduleControlNoticeText}</span>
    <button aria-label="Dismiss module control notice" onClick={onDismiss}><X size={17} /></button>
  </div>;
}
