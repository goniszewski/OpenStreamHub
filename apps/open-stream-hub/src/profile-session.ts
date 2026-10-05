import type { AppBootstrap, BootstrapError, Gesture, KeypadPreviewResult, Profile, ProfileListing, ProfileSummary, ReplayEvent, ReplayKind, ReplayResult, ServiceModule, StoredProfile, UnreadableProfile } from './contracts';
import { closeDecision, stableStringify } from './draft-model';
import { keypadPagesAtPath, upgradeKeypadProfile } from './keypad-model';
import { errorMessage, faderMaximum, replayLabels } from './ui-model';

export type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export type ReplaySession = { status: 'idle' } | { status: 'running'; name: string } | { status: 'failed'; name: string; error: string } |
  { status: 'completed'; name: string; result: ReplayResult };
export type Preview = { status: 'idle' } | { status: 'running' } | { status: 'failed'; error: string } | { status: 'completed'; event: ReplayEvent };
export type KeypadPreview = { status: 'idle' } | { status: 'running' } | { status: 'failed'; error: string } | { status: 'completed'; result: KeypadPreviewResult };
export type Validation = 'idle' | 'running' | 'valid' | 'failed';
export interface PendingIntent { label: string; run: () => void }

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
  // Increments whenever a different stored or new profile replaces the draft,
  // so view state such as the open keypad folder can reset.
  draftGeneration: number;
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

export interface BindingPreviewRequest { module: ServiceModule; control: number; gesture: Gesture; serial: string; faderPosition: number }
export interface KeypadPreviewRequest { path: string[]; control: number; serial: string }

// Owns the profile draft and every request to the Rust service. Each request
// kind has a sequence number; a response is applied only if no newer request
// or edit has superseded it.
export class ProfileSession {
  private state: SessionState;
  private readonly listeners = new Set<() => void>();
  private readonly sequence = { bootstrap: 0, load: 0, list: 0, replay: 0, preview: 0, validation: 0 };

  constructor(private readonly invoke: Invoke, initial: Profile, desktop: boolean) {
    const draft = newDraft(initial);
    this.state = {
      draft: { ...draft, savedSnapshot: stableStringify(draft.profile) }, draftGeneration: 0,
      profiles: [], unreadable: [], storagePath: '', ready: !desktop, bootstrapError: null,
      busy: desktop ? 'Opening profiles' : null, error: null, notice: null, validation: 'idle',
      replay: { status: 'idle' }, preview: { status: 'idle' }, keypadPreview: { status: 'idle' }, pendingIntent: null,
    };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<SessionState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }

  get unsaved() { return isUnsaved(this.state.draft); }

  private invalidateResults(): Partial<SessionState> {
    this.sequence.replay += 1;
    this.sequence.preview += 1;
    this.sequence.validation += 1;
    this.sequence.list += 1;
    return { replay: { status: 'idle' }, preview: { status: 'idle' }, keypadPreview: { status: 'idle' }, validation: 'idle', notice: null, error: null };
  }

  private adopt(stored: StoredProfile): Partial<SessionState> {
    return { ...this.invalidateResults(), draft: storedDraft(stored), draftGeneration: this.state.draftGeneration + 1 };
  }

  setError(error: string | null) { this.set({ error }); }
  setNotice(notice: string | null) { this.set({ notice }); }

  // Replaces the draft profile. Results computed from the old draft are discarded.
  edit(profile: Profile) { this.set({ ...this.invalidateResults(), draft: { ...this.state.draft, profile } }); }

  // Applies an edit to the latest draft, not to a value captured earlier.
  update(change: (profile: Profile) => Profile) { this.edit(change(this.state.draft.profile)); }

  newProfile(profile: Profile = emptyProfile) { this.set({ ...this.invalidateResults(), draft: newDraft(profile), draftGeneration: this.state.draftGeneration + 1 }); }

  // Runs immediately, or asks before discarding an unsaved draft.
  guard(label: string, run: () => void) {
    if (this.unsaved) this.set({ pendingIntent: { label, run } }); else run();
  }

  resolveIntent(discard: boolean) {
    const intent = this.state.pendingIntent;
    this.set({ pendingIntent: null });
    if (discard) intent?.run();
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
      this.set({
        ...this.invalidateResults(), draft, draftGeneration: this.state.draftGeneration + 1, profiles: value.profiles, unreadable: value.unreadable,
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
      this.set({ ...this.adopt(stored), notice: `Loaded ${stored.profile.name}.` });
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
