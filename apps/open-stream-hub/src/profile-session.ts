import type { AppBootstrap, BootstrapError, Gesture, KeypadPreviewResult, Profile, ProfileListing, ProfileSummary, ReplayEvent, ReplayKind, ReplayResult, ServiceModule, StoredProfile, UnreadableProfile } from './contracts';
import { closeDecision, stableStringify } from './draft-model';
import { emptyHistory, recordEdit, redoStep, undoStep } from './edit-history';
import type { History } from './edit-history';
import { newDraftKey, restoreDecision } from './draft-store';
import type { DraftPersister, StoredDraft } from './draft-store';
import { copyName, ImportError, importName, parseImportedProfile, withName } from './profile-library';
import { keypadPagesAtPath, upgradeKeypadProfile } from './keypad-model';
import { errorMessage, faderMaximum, replayLabels } from './ui-model';

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
}

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
  exportResult: ExportResult | null;
  // Increments whenever a different stored or new profile replaces the draft,
  // so view state such as the open keypad folder can reset.
  draftGeneration: number;
  // Whether the bounded edit history has a step to undo or redo.
  canUndo: boolean;
  canRedo: boolean;
}

export const emptyProfile: Profile = { schema_version: 1, name: 'New profile', bindings: [] };

export function isUnsaved(draft: Draft) {
  return stableStringify(draft.profile) !== draft.savedSnapshot;
}

export function newDraft(profile: Profile): Draft {
  return { profile: upgradeKeypadProfile(profile), profileId: null, savedSnapshot: '', modifiedAtMs: null };
}

function storedDraft(stored: StoredProfile): Draft {
  const profile = upgradeKeypadProfile(stored.profile);
  return { profile, profileId: stored.id, savedSnapshot: stableStringify(profile), modifiedAtMs: stored.modified_at_ms };
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
  private readonly sequence = { bootstrap: 0, load: 0, list: 0, replay: 0, preview: 0, validation: 0 };
  private history: History<Profile> = emptyHistory();

  constructor(private readonly invoke: Invoke, initial: Profile, desktop: boolean, private readonly drafts: DraftPersister | null = null, private readonly now: () => number = Date.now) {
    if (drafts) drafts.onOversize = () => this.setNotice('This draft is too large for a crash-recovery copy. Save it to keep your changes.');
    const draft = newDraft(initial);
    this.state = {
      draft: { ...draft, savedSnapshot: stableStringify(draft.profile) }, draftGeneration: 0,
      profiles: [], unreadable: [], storagePath: '', ready: !desktop, bootstrapError: null,
      busy: desktop ? 'Opening profiles' : null, error: null, notice: null, validation: 'idle',
      replay: { status: 'idle' }, preview: { status: 'idle' }, keypadPreview: { status: 'idle' }, pendingIntent: null,
      duplicatePrompt: false, restorePrompt: null, importError: null, exportResult: null, canUndo: false, canRedo: false,
    };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<SessionState>) {
    const previous = this.state.draft;
    this.state = { ...this.state, ...patch };
    if (this.state.draft !== previous) this.observeDraft();
    for (const listener of this.listeners) listener();
  }

  get unsaved() { return isUnsaved(this.state.draft); }

  // Mirrors the draft into crash-recovery storage.
  private observeDraft() {
    const { profile, profileId, modifiedAtMs } = this.state.draft;
    const worthKeeping = this.unsaved && (profileId !== null || stableStringify(profile) !== stableStringify(upgradeKeypadProfile(emptyProfile)));
    this.drafts?.observe({ key: profileId ?? newDraftKey, profile, baseModifiedAtMs: modifiedAtMs, dirty: worthKeeping });
  }

  flushDraft() { this.drafts?.flush(); }

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
      this.set({ ...this.invalidateResults(), ...history, restorePrompt: null, draft: newDraft(prompt.draft.profile), draftGeneration: this.state.draftGeneration + 1, notice: 'Restored your unsaved new profile.' });
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
  private resetHistory(): Pick<SessionState, 'canUndo' | 'canRedo'> {
    this.history = emptyHistory();
    return { canUndo: false, canRedo: false };
  }

  private adopt(stored: StoredProfile): Partial<SessionState> {
    return { ...this.invalidateResults(), ...this.resetHistory(), draft: storedDraft(stored), draftGeneration: this.state.draftGeneration + 1 };
  }

  setError(error: string | null) { this.set({ error }); }
  setNotice(notice: string | null) { this.set({ notice }); }

  // Replaces the draft profile and records the change for undo. Results computed
  // from the old draft are discarded. An optional `key` names the edited field:
  // rapid consecutive edits with the same key become one undo step.
  edit(profile: Profile, key?: string) {
    this.history = recordEdit(this.history, this.state.draft.profile, profile, { key, at: this.now() });
    this.set({ ...this.invalidateResults(), ...this.historyFlags(), draft: { ...this.state.draft, profile } });
  }

  // Applies an edit to the latest draft, not to a value captured earlier.
  update(change: (profile: Profile) => Profile, key?: string) { this.edit(change(this.state.draft.profile), key); }

  private historyFlags() { return { canUndo: this.history.past.length > 0, canRedo: this.history.future.length > 0 }; }

  // Undo and redo move only the draft. The saved profile and its file are never
  // touched, and the dirty state follows the draft (undoing back to the saved
  // profile makes it clean). They wait while a profile operation is running.
  undo(): boolean { return this.travel(undoStep(this.history, this.state.draft.profile)); }
  redo(): boolean { return this.travel(redoStep(this.history, this.state.draft.profile)); }

  private travel(step: { history: History<Profile>; value: Profile } | null): boolean {
    if (!step || this.state.busy !== null) return false;
    this.history = step.history;
    this.set({ ...this.invalidateResults(), ...this.historyFlags(), draft: { ...this.state.draft, profile: step.value } });
    return true;
  }

  newProfile(profile: Profile = emptyProfile) { this.set({ ...this.invalidateResults(), ...this.resetHistory(), draft: newDraft(profile), draftGeneration: this.state.draftGeneration + 1 }); }

  // Runs immediately, or asks before discarding an unsaved draft.
  guard(label: string, run: () => void) {
    if (this.unsaved) this.set({ pendingIntent: { label, run } }); else run();
  }

  resolveIntent(discard: boolean) {
    const intent = this.state.pendingIntent;
    this.set({ pendingIntent: null });
    if (discard) { this.drafts?.discardCurrent(); intent?.run(); }
  }

  // Returns true if the window may close now.
  requestClose(destroy: () => void): boolean {
    const decision = closeDecision(this.state.busy !== null, this.unsaved);
    if (decision === 'wait') this.set({ error: 'Wait for the profile operation to finish before closing the window.' });
    if (decision === 'confirm') this.set({ pendingIntent: { label: 'close the window', run: destroy } });
    return decision === 'allow';
  }

  async bootstrap() {
    const sequence = ++this.sequence.bootstrap;
    this.set({ busy: 'Opening profiles', bootstrapError: null, error: null });
    try {
      const value = await this.invoke<AppBootstrap>('bootstrap');
      if (sequence !== this.sequence.bootstrap) return;
      const draft = value.profile ? storedDraft(value.profile) : newDraft(emptyProfile);
      this.drafts?.store.purge(value.profiles.map(item => item.id));
      let restorePrompt = this.restoreOffer(draft);
      // A never-saved draft is offered only when the opened profile has nothing to restore.
      if (!restorePrompt && value.profile) restorePrompt = this.restoreOffer(newDraft(emptyProfile));
      this.set({
        ...this.invalidateResults(), ...this.resetHistory(), draft, restorePrompt, draftGeneration: this.state.draftGeneration + 1, profiles: value.profiles, unreadable: value.unreadable,
        storagePath: value.storage_path, ready: true, busy: null,
        notice: value.profile ? null : 'No readable profiles were found. Unreadable files are kept unchanged; save this new profile to continue.',
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
      this.set({ profiles: listing.profiles, unreadable: listing.unreadable, notice: 'Profile list refreshed. Your draft is unchanged.' });
    } catch (reason) { if (sequence === this.sequence.list) this.set({ error: `Could not list profiles: ${errorMessage(reason)}` }); }
  }

  private async refreshAfterChange(prefix: string) {
    const sequence = this.sequence.list + 1;
    try {
      const listing = await this.fetchListing();
      if (listing) this.set({ profiles: listing.profiles, unreadable: listing.unreadable });
    } catch (reason) {
      if (sequence === this.sequence.list) this.set({ error: `${prefix}, but the profile list could not refresh: ${errorMessage(reason)}` });
    }
  }

  async save() {
    if (!this.state.ready || this.state.busy !== null) return;
    const { profile, profileId, modifiedAtMs } = this.state.draft;
    const snapshot = structuredClone(profile);
    this.sequence.list += 1;
    this.set({ busy: 'Saving profile', error: null, notice: null });
    try {
      const stored = await this.invoke<StoredProfile>('save_profile', { id: profileId, profile: snapshot, expectedModifiedAtMs: profileId ? modifiedAtMs : null });
      // Saving keeps the open keypad folder, so the generation is unchanged.
      this.set({ ...this.adopt(stored), draftGeneration: this.state.draftGeneration, validation: 'valid', notice: `Saved ${stored.profile.name}.` });
      await this.refreshAfterChange('Profile saved');
    } catch (reason) { this.set({ error: `Could not save profile: ${errorMessage(reason)}`, validation: 'failed' }); }
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
        draft: deletedOpen ? { ...draft, profileId: null, savedSnapshot: '', modifiedAtMs: null } : draft,
        profiles: this.state.profiles.filter(item => item.id !== summary.id),
        notice: deletedOpen ? `Deleted ${summary.name}. Its contents remain open as an unsaved draft.` : `Deleted ${summary.name}.`,
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
  ): Promise<string | null> {
    if (!this.state.ready || this.state.busy !== null) return 'Wait for the current profile operation to finish.';
    this.sequence.list += 1;
    this.set({ busy, error: null, notice: null });
    try {
      const original = await source();
      const listing = await this.invoke<ProfileListing>('list_profiles');
      const profile = withName(upgradeKeypadProfile(structuredClone(original)), nameFor(original.name, listing.profiles.map(item => item.name)));
      const stored = await this.invoke<StoredProfile>('save_profile', { id: null, profile, expectedModifiedAtMs: null });
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
    const failure = await this.createProfile(read, 'Duplicating profile', copyName, name => `Created ${name}. It is now the open profile.`);
    if (failure) this.set({ error: `Could not duplicate profile: ${failure}` });
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
      void this.createProfile(() => profile, 'Importing profile', importName, name => `Imported ${name}. It is now the open profile.`)
        .then(failure => { if (failure) this.set({ importError: `Could not import the profile: ${failure}` }); });
    });
  }

  async exportProfile() {
    if (!this.state.ready || this.state.busy !== null) return;
    const snapshot = structuredClone(this.state.draft.profile);
    this.set({ busy: 'Exporting profile', error: null, notice: null, exportResult: null });
    try {
      const exportResult = await this.invoke<ExportResult>('export_profile', { profile: snapshot });
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
