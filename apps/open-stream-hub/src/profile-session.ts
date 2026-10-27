import type { AppBootstrap, BootstrapError, Gesture, KeypadPreviewResult, MhzImportResult, Profile, ProfileListing, ProfileSummary, ProfileVersion, ReplayEvent, ReplayKind, ReplayResult, ServiceModule, StoredProfile, UnreadableProfile } from './contracts';
import { profileIssue, stableStringify } from './draft-model';
import { emptyHistory, recentRedoLabels, recentUndoLabels, recordEdit, redoStep, undoStep } from './edit-history';
import { describeEdit } from './edit-labels';
import type { History } from './edit-history';
import { newDraftKey, restoreDecision, timeoutSchedule } from './draft-store';
import type { Cancel, DraftPersister, Schedule, StoredDraft } from './draft-store';
import { copyName, ImportError, importName, parseImportedProfile, starterName, withName } from './profile-library';
import { keypadPagesAtPath, upgradeKeypadProfile } from './keypad-model';
import type { ApplySavedResult } from './service-runner';
import { errorMessage, faderMaximum, replayLabels, versionTimeLabel } from './ui-model';

// How long an edit settles before it is saved to the profile file. The
// crash-recovery copy in local storage is written sooner (draftWriteDelayMs);
// this is the distance to the real file.
export const autosaveDelayMs = 1000;

export type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export type ReplaySession = { status: 'idle' } | { status: 'running'; name: string } | { status: 'failed'; name: string; error: string } |
  { status: 'completed'; name: string; result: ReplayResult };
export type Preview = { status: 'idle' } | { status: 'running' } | { status: 'failed'; error: string } | { status: 'completed'; event: ReplayEvent };
export type KeypadPreview = { status: 'idle' } | { status: 'running' } | { status: 'failed'; error: string } | { status: 'completed'; result: KeypadPreviewResult };
export type Validation = 'idle' | 'running' | 'valid' | 'failed';
export interface PendingIntent { label: string; run: () => void }
export interface ExportResult { path: string; bytes: number }
export interface RestorePrompt { key: string; name: string; changedOnDisk: boolean; draft: StoredDraft }

export interface Draft {
  profile: Profile;
  profileId: string | null;
  // stableStringify of the stored profile; '' for a never-saved draft.
  savedSnapshot: string;
  // Modification time the draft was loaded at, sent with saves for conflict detection.
  modifiedAtMs: number | null;
  // The saved profile this draft was opened from. Unlike profileId it survives deleting that profile, so the backend can keep
  // a derivative of an imported dry-run-only profile dry run only (sourceProfileId / source_profile_id).
  sourceId?: string | null;
  // StoredProfile.content_hash of the saved version, named when the user allows an imported profile.
  contentHash?: string | null;
}

// An action offered beside a notice, such as Undo after a drop replaced a binding.
export interface NoticeAction { label: string; run(): void }

export interface SessionState {
  draft: Draft;
  profiles: ProfileSummary[];
  unreadable: UnreadableProfile[];
  storagePath: string;
  ready: boolean;
  bootstrapError: { message: string; storagePath: string | null } | null;
  busy: string | null;
  error: string | null;
  notice: string | null;
  noticeAction: NoticeAction | null;
  validation: Validation;
  replay: ReplaySession;
  preview: Preview;
  keypadPreview: KeypadPreview;
  pendingIntent: PendingIntent | null;
  // Set while a dirty saved profile is being duplicated and the source is undecided.
  duplicatePrompt: boolean;
  // A crash-recovery copy of unsaved changes that the user has not yet decided on.
  restorePrompt: RestorePrompt | null;
  importError: string | null;
  // The result of the last official .mhz import (kept for this session) and whether its report is open.
  mhzImport: MhzImportResult | null;
  mhzReportOpen: boolean;
  // The imported profile list is missing or damaged: every profile runs only as a dry run until marked for review.
  importFlagsError: string | null;
  exportResult: ExportResult | null;
  // Increments whenever a different stored or new profile replaces the draft,
  // so view state such as the open keypad folder can reset.
  draftGeneration: number;
  // The bounded edit history has a step to undo or redo.
  canUndo: boolean;
  // What the next undos would reverse and the next redos repeat, most recent first (at most ten), for the history list.
  undoLabels: string[];
  redoLabels: string[];
  canRedo: boolean;
  // The automatic-saving setting (default on; persisted in local storage by App).
  autosave: boolean;
  // Set when automatic saving hit a disk conflict and stopped for this draft,
  // so it cannot loop errors. A reload or a successful save clears it.
  autosaveBlocked: boolean;
  // The earlier versions of `versionsId` (newest first), listed on demand for
  // the profile manager sheet. Null while nothing was fetched for that profile.
  versions: ProfileVersion[] | null;
  versionsId: string | null;
  versionsError: string | null;
}

export const emptyProfile: Profile = { schema_version: 1, name: 'New profile', bindings: [] };

// Whether the Run panel offers only a dry run for the open draft (convenience only; the backend decides).
export function draftDryRunOnly(state: Pick<SessionState, 'importFlagsError' | 'profiles' | 'unreadable' | 'draft'>): boolean {
  return draftRunBlock(state) !== null;
}

export type RunBlock = 'imported' | 'deleted' | 'unreadable';

// Why Execute is unavailable for the open draft, or null: the imported-profile list is damaged, the draft comes from a
// dry-run-only profile, or its saved profile was deleted (unknown here, so Execute stays off until it is saved; an
// unreadable saved profile still counts as present).
export function draftRunBlock(state: Pick<SessionState, 'importFlagsError' | 'profiles' | 'unreadable' | 'draft'>): RunBlock | null {
  if (state.importFlagsError !== null) return 'unreadable';
  const { profileId, sourceId } = state.draft;
  if (state.profiles.some(item => (item.id === profileId || item.id === sourceId) && item.dry_run_only)) return 'imported';
  const known = (id: string) => state.profiles.some(item => item.id === id) || state.unreadable.some(item => item.id === id);
  if (typeof sourceId === 'string' && !known(sourceId)) return 'deleted';
  return null;
}

export function isUnsaved(draft: Draft) {
  return stableStringify(draft.profile) !== draft.savedSnapshot;
}

export function newDraft(profile: Profile): Draft {
  return { profile: upgradeKeypadProfile(profile), profileId: null, savedSnapshot: '', modifiedAtMs: null };
}

function storedDraft(stored: StoredProfile): Draft {
  const profile = upgradeKeypadProfile(stored.profile);
  return { profile, profileId: stored.id, savedSnapshot: stableStringify(profile), modifiedAtMs: stored.modified_at_ms, sourceId: stored.id, contentHash: stored.content_hash ?? null };
}

function bootstrapFailure(reason: unknown): { message: string; storagePath: string | null } {
  if (reason && typeof reason === 'object' && 'message' in reason && typeof (reason as BootstrapError).message === 'string') {
    const error = reason as BootstrapError;
    return { message: error.message, storagePath: typeof error.storage_path === 'string' ? error.storage_path : null };
  }
  return { message: errorMessage(reason), storagePath: null };
}

// dialFunction selects the M131E dial function to preview; omitted, the service previews the first one.
export interface BindingPreviewRequest { module: ServiceModule; control: number; gesture: Gesture; serial: string; faderPosition: number; dialFunction?: number }
export interface KeypadPreviewRequest { path: string[]; control: number; serial: string }

// Owns the profile draft and every request to the Rust service. Each request
// kind has a sequence number; a response is applied only if no newer request
// or edit has superseded it.
export class ProfileSession {
  private state: SessionState;
  private readonly listeners = new Set<() => void>();
  private readonly sequence = { bootstrap: 0, load: 0, list: 0, replay: 0, preview: 0, validation: 0, versions: 0 };
  private history: History<Profile> = emptyHistory();
  private readonly desktop: boolean;
  // The pending automatic save: one timer, one in-flight request. The edit
  // history and every result stay live across an automatic save; only the
  // saved baseline moves.
  private autosaveTimer: { cancel: Cancel } | null = null;
  private autosaveInFlight = false;
  // Called after a save of a saved profile: the running sessions of that
  // profile swap to the saved content in place. The app points this at the
  // run store once it exists; without it a save simply applies at the next
  // start.
  private applySaved: ((id: string) => Promise<ApplySavedResult>) | null = null;

  setApplySaved(apply: ((id: string) => Promise<ApplySavedResult>) | null) {
    this.applySaved = apply;
  }

  // What a save changed in the runs that hold the profile, as the notice's
  // tail: swapped live, paused for one confirmation of the additions, or
  // kept back with a reason. Null when nothing runs the profile, or nothing
  // needs saying (an automatic save stays silent unless it does).
  private async savedRunTail(id: string): Promise<string | null> {
    if (!this.applySaved) return null;
    let result: ApplySavedResult;
    try {
      result = await this.applySaved(id);
    } catch (reason) {
      return ` The running profile could not switch: ${errorMessage(reason)}`;
    }
    if (!result.ok) return ` The running profile could not switch: ${result.error}`;
    const refused = result.refused.length ? ` The running profile kept the previous version: ${result.refused.join(' ')}` : '';
    if (result.paused > 0) {
      const runs = result.paused === 1 ? 'One run pauses' : `${result.paused} runs pause`;
      return ` ${runs} until you confirm the changes: press Resume in the header.${refused}`;
    }
    if (result.switched > 0 && !refused) return ' The running profile switched to it.';
    return refused || null;
  }

  constructor(private readonly invoke: Invoke, initial: Profile, desktop: boolean, private readonly drafts: DraftPersister | null = null, private readonly now: () => number = Date.now, private readonly autosaveSchedule: Schedule = timeoutSchedule) {
    this.desktop = desktop;
    if (drafts) drafts.onOversize = () => this.setNotice('This draft is too large for a crash-recovery copy. Save it to keep your changes.');
    const draft = newDraft(initial);
    this.state = {
      draft: { ...draft, savedSnapshot: stableStringify(draft.profile) }, draftGeneration: 0,
      profiles: [], unreadable: [], storagePath: '', ready: !desktop, bootstrapError: null,
      busy: desktop ? 'Opening profiles' : null, error: null, notice: null, noticeAction: null, validation: 'idle',
      replay: { status: 'idle' }, preview: { status: 'idle' }, keypadPreview: { status: 'idle' }, pendingIntent: null,
      duplicatePrompt: false, restorePrompt: null, importError: null, mhzImport: null, mhzReportOpen: false, importFlagsError: null, exportResult: null, canUndo: false, canRedo: false, undoLabels: [], redoLabels: [],
      autosave: true, autosaveBlocked: false, versions: null, versionsId: null, versionsError: null,
    };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<SessionState>) {
    // A notice write drops a stale notice action unless it carries a fresh one.
    const cleared = 'notice' in patch && patch.noticeAction === undefined ? { noticeAction: null } : {};
    const previous = this.state.draft;
    this.state = { ...this.state, ...cleared, ...patch };
    if (this.state.draft !== previous) this.observeDraft();
    for (const listener of this.listeners) listener();
  }

  get unsaved() { return isUnsaved(this.state.draft); }

  // Mirrors the draft into crash-recovery storage and follows it with the
  // automatic save: scheduled for a dirty saved profile, cancelled once the
  // draft is clean again (undone back to the baseline, or saved).
  private observeDraft() {
    const { profile, profileId, modifiedAtMs, sourceId } = this.state.draft;
    const worthKeeping = this.unsaved && (profileId !== null || stableStringify(profile) !== stableStringify(upgradeKeypadProfile(emptyProfile)));
    if (profileId) this.drafts?.store.setLastOpened(profileId);
    this.drafts?.observe({ key: profileId ?? newDraftKey, profile, baseModifiedAtMs: modifiedAtMs, dirty: worthKeeping, sourceId: sourceId ?? null });
    this.syncAutosaveTimer();
  }

  flushDraft() { this.drafts?.flush(); }

  // The automatic save applies to a saved profile with edits that can be
  // saved. A new profile has no file yet and keeps the crash-recovery copy
  // and its restore offer; an invalid draft (a name still being retyped, for
  // example) stays a local recovery copy too — the field's validation
  // message says why, and the next edit re-checks.
  private autosaveEligible(): boolean {
    const state = this.state;
    return state.autosave && !state.autosaveBlocked && this.desktop && state.ready
      && state.draft.profileId !== null && this.unsaved && profileIssue(state.draft.profile) === null;
  }

  // Exactly one timer at a time; a scheduled save is not delayed further by
  // more edits, and a clean draft cancels the pending one.
  private syncAutosaveTimer() {
    if (!this.autosaveEligible()) {
      this.autosaveTimer?.cancel();
      this.autosaveTimer = null;
      return;
    }
    if (this.autosaveTimer || this.autosaveInFlight) return;
    const timer = { cancel: this.autosaveSchedule(() => { this.autosaveTimer = null; void this.runAutosave(); }, autosaveDelayMs) };
    this.autosaveTimer = timer;
  }

  setAutosave(on: boolean) {
    this.set({ autosave: on });
    this.syncAutosaveTimer();
  }

  // Saves the pending edit now instead of after the debounce: the guard, the
  // window close and starting a run all settle the draft first. Resolves to
  // true when the draft is saved afterwards.
  async flushAutosave(): Promise<boolean> {
    this.autosaveTimer?.cancel();
    this.autosaveTimer = null;
    if (!this.unsaved) return true;
    if (!this.autosaveEligible()) return false;
    return this.runAutosave();
  }

  private async runAutosave(): Promise<boolean> {
    if (!this.autosaveEligible()) return false;
    // A manual save or another operation is running; try again after the
    // debounce instead of failing against it.
    if (this.state.busy !== null) { this.syncAutosaveTimer(); return false; }
    const { profile, profileId, modifiedAtMs, sourceId } = this.state.draft;
    const snapshot = stableStringify(profile);
    this.autosaveInFlight = true;
    try {
      const stored = await this.invoke<StoredProfile>('save_profile', { request: { id: profileId, profile: structuredClone(profile), expectedModifiedAtMs: modifiedAtMs, sourceProfileId: sourceId ?? null } });
      // An edit that landed while the request was out has already
      // rescheduled; the response then only confirms the older file, and the
      // draft stays unsaved so a guard still asks before discarding it.
      if (stableStringify(this.state.draft.profile) === snapshot) {
        this.set({ draft: { ...storedDraft(stored) }, autosaveBlocked: false });
        await this.refreshAfterChange('Profile saved');
        // The automatic save stays silent unless a run needs attention.
        const tail = await this.savedRunTail(stored.id);
        if (tail !== null) this.set({ notice: `Saved ${stored.profile.name}.${tail}` });
      }
    } catch (reason) {
      const message = errorMessage(reason);
      const conflict = message.includes('changed on disk');
      this.set({ error: `Could not save automatically: ${message}`, ...(conflict ? { autosaveBlocked: true } : {}) });
    } finally {
      this.autosaveInFlight = false;
      this.syncAutosaveTimer();
    }
    return !this.unsaved;
  }

  // Looks for a stored draft of the profile just opened. Holds its key until
  // the user decides, so it is not cleared by the draft becoming current.
  private restoreOffer(draft: Draft): RestorePrompt | null {
    if (!this.drafts) return null;
    const key = draft.profileId ?? newDraftKey;
    const stored = this.drafts.store.read(key);
    const decision = restoreDecision(stored, draft.profileId ? { profile: draft.profile, modifiedAtMs: draft.modifiedAtMs } : null);
    if (decision.kind === 'none' || !stored) { if (stored) this.drafts.store.clear(key); return null; }
    this.drafts.hold(key);
    return { key, name: stored.profile.name, changedOnDisk: decision.changedOnDisk, draft: stored };
  }

  resolveRestore(restore: boolean) {
    const prompt = this.state.restorePrompt;
    if (!prompt) return;
    this.drafts?.release(prompt.key);
    const draft = this.state.draft;
    const history = this.resetHistory();
    if (!restore) { this.drafts?.store.clear(prompt.key); this.set({ restorePrompt: null, ...history }); return; }
    if (prompt.key === newDraftKey) {
      // A recovered never-saved draft keeps the source it came from (a deleted profile, for example).
      this.set({ ...this.invalidateResults(), ...history, restorePrompt: null, draft: { ...newDraft(prompt.draft.profile), sourceId: prompt.draft.source_id ?? null }, draftGeneration: this.state.draftGeneration + 1, notice: 'Restored your unsaved new profile.' });
    } else if (draft.profileId === prompt.key) {
      // Keeping the stored base time makes a save conflict if the file changed since.
      this.set({
        ...this.invalidateResults(), ...history, restorePrompt: null, draftGeneration: this.state.draftGeneration + 1,
        draft: { ...draft, profile: prompt.draft.profile, modifiedAtMs: prompt.draft.base_modified_at_ms ?? draft.modifiedAtMs },
        notice: prompt.changedOnDisk ? `Restored unsaved changes to ${prompt.name}. The saved profile changed on disk since, so saving will ask you to reload first.` : `Restored unsaved changes to ${prompt.name}.`,
      });
    } else this.set({ restorePrompt: null, ...history });
  }

  private invalidateResults(): Partial<SessionState> {
    this.sequence.replay += 1;
    this.sequence.preview += 1;
    this.sequence.validation += 1;
    this.sequence.list += 1;
    return { replay: { status: 'idle' }, preview: { status: 'idle' }, keypadPreview: { status: 'idle' }, validation: 'idle', notice: null, error: null, exportResult: null, importError: null };
  }

  // A different profile, or a saved baseline, replaces the draft: the edit
  // history belongs to the old draft and is cleared.
  private resetHistory(): Pick<SessionState, 'canUndo' | 'canRedo' | 'undoLabels' | 'redoLabels'> {
    this.history = emptyHistory();
    return { canUndo: false, canRedo: false, undoLabels: [], redoLabels: [] };
  }

  private adopt(stored: StoredProfile): Partial<SessionState> {
    return { ...this.invalidateResults(), ...this.resetHistory(), draft: storedDraft(stored), draftGeneration: this.state.draftGeneration + 1, autosaveBlocked: false };
  }

  // A new profile opens in the editor only. Choosing it in the profile picker is what runs it on the modules.
  private openNote() { return this.desktop ? 'It is open in the editor, not running on the modules yet. Choose it in the profile picker to run it.' : 'It is now the open profile.'; }
  setError(error: string | null) { this.set({ error }); }
  setNotice(notice: string | null, action?: NoticeAction) { this.set({ notice, ...(action ? { noticeAction: action } : {}) }); }

  // Replaces the draft profile and records the change for undo. Results computed
  // from the old draft are discarded. An optional `key` names the edited field:
  // rapid consecutive edits with the same key become one undo step.
  edit(profile: Profile, key?: string) {
    this.history = recordEdit(this.history, this.state.draft.profile, profile, { key, at: this.now() }, undefined, describeEdit);
    this.set({ ...this.invalidateResults(), ...this.historyFlags(), draft: { ...this.state.draft, profile } });
  }

  // Applies an edit to the latest draft, not to a value captured earlier.
  update(change: (profile: Profile) => Profile, key?: string) { this.edit(change(this.state.draft.profile), key); }

  private historyFlags() {
    return {
      canUndo: this.history.past.length > 0, canRedo: this.history.future.length > 0,
      undoLabels: recentUndoLabels(this.history), redoLabels: recentRedoLabels(this.history),
    };
  }

  // Undo and redo move only the draft. The saved profile and its file are never
  // touched, and the dirty state follows the draft (undoing back to the saved
  // profile makes it clean). They wait while a profile operation is running.
  undo(): boolean { return this.travel(undoStep(this.history, this.state.draft.profile)); }
  redo(): boolean { return this.travel(redoStep(this.history, this.state.draft.profile)); }
  // Goes back (or forward) `count` steps in one go, as picking an entry in the history list does; stops where the
  // history ends or a profile operation is running. Returns how many steps were taken.
  undoSteps(count: number): number { let taken = 0; while (taken < count && this.undo()) taken += 1; return taken; }
  redoSteps(count: number): number { let taken = 0; while (taken < count && this.redo()) taken += 1; return taken; }

  private travel(step: { history: History<Profile>; value: Profile } | null): boolean {
    if (!step || this.state.busy !== null) return false;
    this.history = step.history;
    this.set({ ...this.invalidateResults(), ...this.historyFlags(), draft: { ...this.state.draft, profile: step.value } });
    return true;
  }

  newProfile(profile: Profile = emptyProfile) { this.set({ ...this.invalidateResults(), ...this.resetHistory(), draft: newDraft(profile), draftGeneration: this.state.draftGeneration + 1, autosaveBlocked: false, versions: null, versionsId: null, versionsError: null }); }

  // Runs immediately, or saves the edit first when automatic saving is on.
  // Only an edit that cannot be saved (automatic saving off, an invalid
  // draft, or a stopped save) still asks before being discarded.
  guard(label: string, run: () => void) {
    if (this.unsaved && this.autosaveEligible()) {
      void this.flushAutosave().then(saved => {
        if (saved) run(); else this.set({ pendingIntent: { label, run } });
      });
      return;
    }
    if (this.unsaved) this.set({ pendingIntent: { label, run } }); else run();
  }

  resolveIntent(discard: boolean) {
    const intent = this.state.pendingIntent;
    this.set({ pendingIntent: null });
    if (discard) { this.drafts?.discardCurrent(); intent?.run(); }
  }

  // Returns true if the window may close now. With automatic saving, an
  // unsaved edit is saved and the window then closes through `destroy`; the
  // caller must keep the window open until that lands (Tauri's close request
  // stays prevented). A save that cannot finish hands over to the usual
  // confirmation instead of closing on a lost edit.
  requestClose(destroy: () => void): boolean {
    if (this.state.busy !== null) {
      this.set({ error: 'Wait for the profile operation to finish before closing the window.' });
      return false;
    }
    if (!this.unsaved) return true;
    if (this.autosaveEligible()) {
      void this.flushAutosave().then(saved => {
        if (saved) destroy(); else this.set({ pendingIntent: { label: 'close the window', run: destroy } });
      });
      return false;
    }
    this.set({ pendingIntent: { label: 'close the window', run: destroy } });
    return false;
  }

  async bootstrap() {
    const sequence = ++this.sequence.bootstrap;
    this.set({ busy: 'Opening profiles', bootstrapError: null, error: null });
    try {
      const value = await this.invoke<AppBootstrap>('bootstrap');
      if (sequence !== this.sequence.bootstrap) return;
      // The store offers the most recently saved profile; come back to the one that was open instead, when it is still there.
      let opened = value.profile;
      const remembered = this.drafts?.store.lastOpened();
      if (opened && remembered && remembered !== opened.id && value.profiles.some(item => item.id === remembered)) {
        try { opened = await this.invoke<StoredProfile>('load_profile', { id: remembered }); } catch { /* the most recent one stays */ }
        if (sequence !== this.sequence.bootstrap) return;
      }
      const draft = opened ? storedDraft(opened) : newDraft(emptyProfile);
      this.drafts?.store.purge(value.profiles.map(item => item.id));
      let restorePrompt = this.restoreOffer(draft);
      // A never-saved draft is offered only when the opened profile has nothing to restore.
      if (!restorePrompt && opened) restorePrompt = this.restoreOffer(newDraft(emptyProfile));
      this.set({
        ...this.invalidateResults(), ...this.resetHistory(), draft, restorePrompt, draftGeneration: this.state.draftGeneration + 1, profiles: value.profiles, unreadable: value.unreadable, importFlagsError: value.import_flags_error ?? null,
        storagePath: value.storage_path, ready: true, busy: null,
        notice: opened ? null : 'No readable profiles were found. Unreadable files are kept unchanged; save this new profile to continue.',
      });
    } catch (reason) {
      if (sequence !== this.sequence.bootstrap) return;
      this.set({ bootstrapError: bootstrapFailure(reason), busy: null });
    }
  }

  async load(id: string) {
    const sequence = ++this.sequence.load;
    this.sequence.list += 1;
    this.set({ busy: 'Loading profile', error: null });
    try {
      const stored = await this.invoke<StoredProfile>('load_profile', { id });
      if (sequence !== this.sequence.load) return;
      const adopted = this.adopt(stored);
      this.set({ ...adopted, restorePrompt: this.restoreOffer(adopted.draft!), notice: `Loaded ${stored.profile.name}.` });
    } catch (reason) {
      if (sequence === this.sequence.load) this.set({ error: `Could not load profile: ${errorMessage(reason)}` });
    } finally { if (sequence === this.sequence.load) this.set({ busy: null }); }
  }

  private async fetchListing(): Promise<ProfileListing | null> {
    const sequence = ++this.sequence.list;
    const listing = await this.invoke<ProfileListing>('list_profiles');
    return sequence === this.sequence.list ? listing : null;
  }

  async refreshProfiles() {
    const sequence = this.sequence.list + 1;
    try {
      const listing = await this.fetchListing();
      if (!listing) return;
      this.set({ profiles: listing.profiles, unreadable: listing.unreadable, importFlagsError: listing.import_flags_error ?? null, notice: 'Profile list refreshed. Your draft is unchanged.' });
    } catch (reason) { if (sequence === this.sequence.list) this.set({ error: `Could not list profiles: ${errorMessage(reason)}` }); }
  }

  private async refreshAfterChange(prefix: string) {
    const sequence = this.sequence.list + 1;
    try {
      const listing = await this.fetchListing();
      if (listing) this.set({ profiles: listing.profiles, unreadable: listing.unreadable, importFlagsError: listing.import_flags_error ?? null });
    } catch (reason) {
      if (sequence === this.sequence.list) this.set({ error: `${prefix}, but the profile list could not refresh: ${errorMessage(reason)}` });
    }
  }

  async save() {
    if (!this.state.ready || this.state.busy !== null) return;
    const { profile, profileId, modifiedAtMs, sourceId } = this.state.draft;
    const snapshot = structuredClone(profile);
    this.sequence.list += 1;
    this.set({ busy: 'Saving profile', error: null, notice: null });
    try {
      const stored = await this.invoke<StoredProfile>('save_profile', { request: { id: profileId, profile: snapshot, expectedModifiedAtMs: profileId ? modifiedAtMs : null, sourceProfileId: sourceId ?? null } });
      // Running profiles of this save switch in place; the notice says what
      // that did. Saving keeps the open keypad folder, so the generation is
      // unchanged.
      const tail = await this.savedRunTail(stored.id);
      this.set({ ...this.adopt(stored), draftGeneration: this.state.draftGeneration, validation: 'valid', notice: `Saved ${stored.profile.name}.${tail ?? ''}` });
      await this.refreshAfterChange('Profile saved');
    } catch (reason) {
      const message = errorMessage(reason);
      // A conflict also stops the pending automatic save, which would refuse
      // with the same message a moment later.
      this.set({ error: `Could not save profile: ${message}`, validation: 'failed', ...(message.includes('changed on disk') ? { autosaveBlocked: true } : {}) });
    }
    finally { this.set({ busy: null }); }
  }

  // The kept earlier versions of a profile, listed when the profile manager sheet
  // opens its "Earlier versions" list. The list follows the open profile.
  async loadVersions(id: string) {
    const sequence = ++this.sequence.versions;
    this.set({ versionsError: null });
    try {
      const versions = await this.invoke<ProfileVersion[]>('profile_versions', { id });
      if (sequence !== this.sequence.versions) return;
      this.set({ versions, versionsId: id });
    } catch (reason) {
      if (sequence === this.sequence.versions) this.set({ versions: null, versionsId: id, versionsError: `Could not list the earlier versions: ${errorMessage(reason)}` });
    }
  }

  // Puts one earlier version back: it is loaded from the version history and
  // then saved as the profile, so the current version is kept first (as a new
  // history entry, whatever the spacing between kept versions) and a change on disk since loading still refuses.
  async restoreVersion(modifiedAtMs: number) {
    const { profileId, modifiedAtMs: loadedAt, sourceId } = this.state.draft;
    if (!profileId || !this.state.ready || this.state.busy !== null) return;
    this.set({ busy: 'Restoring version', error: null, notice: null });
    try {
      const version = await this.invoke<StoredProfile>('load_profile_version', { id: profileId, modifiedAtMs });
      const stored = await this.invoke<StoredProfile>('save_profile', { request: { id: profileId, profile: version.profile, expectedModifiedAtMs: loadedAt, sourceProfileId: sourceId ?? null, keepVersion: true } });
      const tail = await this.savedRunTail(stored.id);
      this.set({ ...this.adopt(stored), draftGeneration: this.state.draftGeneration, validation: 'valid', versions: null, versionsId: null, notice: `Restored the version from ${versionTimeLabel(version.modified_at_ms, this.now())}.${tail ?? ''}` });
      await this.refreshAfterChange('Version restored');
    } catch (reason) {
      const message = errorMessage(reason);
      // A conflict also stops the pending automatic save, which would refuse
      // with the same message a moment later.
      this.set({ error: `Could not restore the version: ${message}`, ...(message.includes('changed on disk') ? { autosaveBlocked: true } : {}) });
    }
    finally { this.set({ busy: null }); }
  }

  // Deleting the open profile keeps its contents as an unsaved draft, so
  // nothing on screen is lost until the user discards it.
  async deleteProfile(summary: ProfileSummary) {
    if (!this.state.ready || this.state.busy !== null) return;
    this.sequence.list += 1;
    this.set({ busy: 'Deleting profile', error: null, notice: null });
    try {
      await this.invoke<null>('delete_profile', { id: summary.id, expectedModifiedAtMs: summary.modified_at_ms });
      const draft = this.state.draft;
      const deletedOpen = draft.profileId === summary.id;
      this.set({
        // The draft keeps sourceId: a later Start or Save still names the deleted source.
        draft: deletedOpen ? { ...draft, profileId: null, savedSnapshot: '', modifiedAtMs: null, contentHash: null } : draft,
        profiles: this.state.profiles.filter(item => item.id !== summary.id),
        notice: deletedOpen ? `Deleted ${summary.name}. Its contents stay open as unsaved changes.` : `Deleted ${summary.name}.`,
      });
      await this.refreshAfterChange('Profile deleted');
    } catch (reason) { this.set({ error: `Could not delete profile: ${errorMessage(reason)}` }); }
    finally { this.set({ busy: null }); }
  }

  async validate() {
    const sequence = ++this.sequence.validation;
    const snapshot = structuredClone(this.state.draft.profile);
    this.set({ validation: 'running', error: null, notice: null });
    try {
      await this.invoke<{ valid: true }>('validate_profile', { profile: snapshot });
      if (sequence === this.sequence.validation) this.set({ validation: 'valid', notice: 'The current draft passes service validation.' });
    } catch (reason) {
      if (sequence === this.sequence.validation) this.set({ validation: 'failed', error: `Profile validation failed: ${errorMessage(reason)}` });
    }
  }

  // Saves, or says briefly why there is nothing to save (the Cmd+S path).
  async saveOrNote() {
    if (!this.unsaved) { this.set({ notice: 'Nothing to save. This profile has no unsaved changes.' }); return; }
    await this.save();
  }

  // Stores a copy of a profile as a NEW saved profile, never overwriting one,
  // and switches to it. Resolves to an error message, or null on success.
  private async createProfile(
    source: () => Promise<Profile> | Profile, busy: string, nameFor: (name: string, existing: string[]) => string, announce: (name: string) => string,
    copyOf: string | null = null,
    // How the new profile reaches the store: an ordinary save by default, or the backend's own creator (a pack's profile).
    create?: (profile: Profile) => Promise<StoredProfile>,
    // A saved profile that already is what this would create: it is opened instead and nothing is saved.
    existing?: (name: string, saved: ProfileListing['profiles']) => ProfileListing['profiles'][number] | undefined,
  ): Promise<string | null> {
    if (!this.state.ready || this.state.busy !== null) return 'Wait for the current profile operation to finish.';
    this.sequence.list += 1;
    this.set({ busy, error: null, notice: null });
    try {
      const original = await source();
      const listing = await this.invoke<ProfileListing>('list_profiles');
      const reused = existing?.(original.name, listing.profiles);
      if (reused) {
        this.set({ busy: null });
        await this.load(reused.id);
        this.set({ notice: `Opened ${reused.name}, which you already have.` });
        return null;
      }
      const profile = withName(upgradeKeypadProfile(structuredClone(original)), nameFor(original.name, listing.profiles.map(item => item.name)));
      // The source lets the backend keep a copy of an imported dry-run-only profile dry run only (it also matches by content).
      const stored = create ? await create(profile) : await this.invoke<StoredProfile>('save_profile', { request: { id: null, profile, expectedModifiedAtMs: null, sourceProfileId: copyOf } });
      this.set({ ...this.adopt(stored), validation: 'valid', notice: announce(stored.profile.name) });
      await this.refreshAfterChange('Profile created');
      return null;
    } catch (reason) { return errorMessage(reason); }
    finally { this.set({ busy: null }); }
  }

  // Duplicates the open profile. With unsaved changes to a saved profile the
  // user chooses which version to copy.
  requestDuplicate() {
    if (!this.state.ready || this.state.busy !== null) return;
    if (this.state.draft.profileId && this.unsaved) this.set({ duplicatePrompt: true });
    else void this.duplicate('draft');
  }

  async resolveDuplicate(source: 'saved' | 'draft' | null) {
    this.set({ duplicatePrompt: false });
    if (source) await this.duplicate(source);
  }

  private async duplicate(source: 'saved' | 'draft') {
    const { profile, profileId } = this.state.draft;
    const read = async () => source === 'saved' && profileId ? upgradeKeypadProfile((await this.invoke<StoredProfile>('load_profile', { id: profileId })).profile) : profile;
    const failure = await this.createProfile(read, 'Duplicating profile', copyName, name => `Created ${name}. ${this.openNote()}`, this.state.draft.sourceId ?? profileId);
    if (failure) this.set({ error: `Could not duplicate profile: ${failure}` });
  }

  // Saves a shipped starter profile as a NEW saved profile (never overwriting
  // one) and switches to it. The guard decides whether unsaved changes must
  // settle first.
  async createFromTemplate(profile: Profile): Promise<string | null> {
    return this.createProfile(() => profile, 'Creating starter profile', starterName, name => `Created ${name} from the starter. ${this.openNote()}`,
      null, undefined, (name, saved) => saved.find(item => item.name === name));
  }

  // Saves an app preset pack's profile as a NEW saved profile that can run only as a dry run until the user allows it
  // to execute actions, and switches to it. The backend flags it before the file is written.
  async createFromPack(profile: Profile): Promise<string | null> {
    return this.createProfile(() => profile, 'Creating profile from a preset', starterName,
      name => `Created ${name} from the preset. It runs as a dry run until you review it and allow it to execute actions in the profile manager.`,
      null, created => this.invoke<StoredProfile>('create_pack_profile', { profile: created }));
  }

  setImportError(importError: string | null) { this.set({ importError }); }

  // Validates file text completely before anything is created or changed.
  async importProfile(text: string) {
    this.set({ importError: null });
    let profile: Profile;
    try {
      profile = parseImportedProfile(text);
      await this.invoke<{ valid: true }>('validate_profile', { profile });
    } catch (reason) {
      this.set({ importError: reason instanceof ImportError ? reason.message : `This file is not a valid profile: ${errorMessage(reason)}` });
      return;
    }
    this.guard('import this profile', () => {
      void this.createProfile(() => profile, 'Importing profile', importName, name => `Imported ${name}. ${this.openNote()}`)
        .then(failure => { if (failure) this.set({ importError: `Could not import the profile: ${failure}` }); });
    });
  }

  // Imports an official MasterHUB .mhz file. The backend reads it in memory within strict limits and creates new
  // profiles that may only run as a dry run until allowed; nothing in the file is executed. The first one opens.
  importMhz(bytes: Uint8Array) {
    this.set({ importError: null });
    this.guard('import this profile', () => { void this.runMhzImport(bytes); });
  }

  private async runMhzImport(bytes: Uint8Array) {
    if (!this.state.ready || this.state.busy !== null) { this.set({ importError: 'Wait for the current profile operation to finish.' }); return; }
    this.sequence.list += 1;
    this.set({ busy: 'Importing official profile', error: null, notice: null });
    try {
      // The raw bytes are the request body (Tauri passes a Uint8Array through unchanged).
      const result = await this.invoke<MhzImportResult>('import_mhz_profile', bytes as unknown as Record<string, unknown>);
      const first = result.profiles[0];
      const names = result.profiles.map(item => item.stored.profile.name).join(', ');
      this.set({ ...(first ? this.adopt(first.stored) : {}), mhzImport: result, mhzReportOpen: true, notice: `Imported ${names}. Imported profiles run only as a dry run until you allow actions.` });
      await this.refreshAfterChange('Profile imported');
    } catch (reason) {
      this.set({ importError: `Could not import the official profile: ${errorMessage(reason)}` });
      // A failed import may still have left a profile it could not remove (named in the error): show the real list.
      await this.refreshAfterChange('The import failed');
    }
    finally { this.set({ busy: null }); }
  }

  // The report stays available for this session; the dry-run banner reopens it.
  dismissMhzImport() { this.set({ mhzReportOpen: false }); }
  showMhzImport() { if (this.state.mhzImport) this.set({ mhzReportOpen: true }); }

  // The explicit "Allow this imported profile to execute actions" step, after the user confirmed the saved version whose
  // capabilities the dialog listed (contentHash). The backend recomputes the hash of the saved file and refuses another
  // version. It cannot prove that a person clicked; the confirmation dialog is that step.
  async allowExecution(id: string, name: string, contentHash: string): Promise<boolean> {
    if (!this.state.ready || this.state.busy !== null) return false;
    this.sequence.list += 1;
    this.set({ busy: 'Allowing actions', error: null, notice: null });
    try {
      const listing = await this.invoke<ProfileListing>('allow_imported_profile_execution', { request: { id, contentHash } });
      this.set({ profiles: listing.profiles, unreadable: listing.unreadable, importFlagsError: listing.import_flags_error ?? null, notice: `${name} may now run with actions. Starting a run with actions still asks for confirmation.` });
      return true;
    } catch (reason) { this.set({ error: `Could not allow actions: ${errorMessage(reason)}` }); return false; }
    finally { this.set({ busy: null }); }
  }

  // Recovery when the imported profile list is missing or damaged: every saved profile becomes dry run only until it is
  // allowed again. Never deletes anything.
  async markAllForReview(): Promise<boolean> {
    if (!this.state.ready || this.state.busy !== null) return false;
    this.sequence.list += 1;
    this.set({ busy: 'Marking profiles for review', error: null, notice: null });
    try {
      const listing = await this.invoke<ProfileListing>('mark_all_profiles_for_review');
      this.set({ profiles: listing.profiles, unreadable: listing.unreadable, importFlagsError: listing.import_flags_error ?? null, notice: 'Every profile now runs only as a dry run until you review it and allow actions.' });
      return true;
    } catch (reason) { this.set({ error: `Could not mark the profiles for review: ${errorMessage(reason)}` }); return false; }
    finally { this.set({ busy: null }); }
  }

  async exportProfile() {
    if (!this.state.ready || this.state.busy !== null) return;
    const snapshot = structuredClone(this.state.draft.profile);
    this.set({ busy: 'Exporting profile', error: null, notice: null, exportResult: null });
    try {
      // The source lets the backend keep an exported edit of an imported dry-run-only profile dry run only when that file is
      // imported again.
      const exportResult = await this.invoke<ExportResult>('export_profile', { request: { profile: snapshot, sourceProfileId: this.state.draft.sourceId ?? null } });
      this.set({ exportResult });
    } catch (reason) { this.set({ error: `Could not export profile: ${errorMessage(reason)}` }); }
    finally { this.set({ busy: null }); }
  }

  // Discards any in-flight preview, for example after the selection changes.
  clearPreview() {
    this.sequence.preview += 1;
    this.set({ preview: { status: 'idle' }, keypadPreview: { status: 'idle' } });
  }

  clearReplay() {
    this.sequence.replay += 1;
    this.set({ replay: { status: 'idle' } });
  }

  async previewBinding(request: BindingPreviewRequest) {
    const sequence = ++this.sequence.preview;
    const snapshot = structuredClone(this.state.draft.profile);
    this.set({ preview: { status: 'running' } });
    try {
      const event = await this.invoke<ReplayEvent>('preview_binding', {
        profile: snapshot, module: request.module, control: request.control, gesture: request.gesture, serialNumber: request.serial || null,
        value: request.module === 'm005s' ? { type: 'position', raw: request.faderPosition, maximum: faderMaximum } : null,
        ...(request.dialFunction === undefined ? {} : { dialFunction: request.dialFunction }),
      });
      if (sequence === this.sequence.preview) this.set({ preview: { status: 'completed', event } });
    } catch (reason) {
      if (sequence === this.sequence.preview) this.set({ preview: { status: 'failed', error: errorMessage(reason) } });
    }
  }

  // Resolves with the result only if it is still current.
  async previewKeypad(request: KeypadPreviewRequest): Promise<KeypadPreviewResult | null> {
    const sequence = ++this.sequence.preview;
    const snapshot = structuredClone(this.state.draft.profile);
    this.set({ keypadPreview: { status: 'running' } });
    try {
      const result = await this.invoke<KeypadPreviewResult>('preview_keypad', { profile: snapshot, path: [...request.path], control: request.control, serialNumber: request.serial || null });
      if (sequence !== this.sequence.preview) return null;
      keypadPagesAtPath(snapshot, result.path);
      this.set({ keypadPreview: { status: 'completed', result } });
      return result;
    } catch (reason) {
      if (sequence === this.sequence.preview) this.set({ keypadPreview: { status: 'failed', error: errorMessage(reason) } });
      return null;
    }
  }

  async replayCapture(kind: ReplayKind) {
    const sequence = ++this.sequence.replay;
    const snapshot = structuredClone(this.state.draft.profile);
    const name = replayLabels[kind];
    this.set({ replay: { status: 'running', name } });
    try {
      const result = await this.invoke<ReplayResult>('replay_demo', { profile: snapshot, kind });
      if (sequence === this.sequence.replay) this.set({ replay: { status: 'completed', name, result } });
    } catch (reason) {
      if (sequence === this.sequence.replay) this.set({ replay: { status: 'failed', name, error: errorMessage(reason) } });
    }
  }
}
