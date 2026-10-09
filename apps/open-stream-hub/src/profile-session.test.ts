import { describe, expect, it } from 'vitest';
import type { AppBootstrap, KeypadPreviewResult, Profile, ProfileListing, ReplayEvent, ReplayResult, StoredProfile } from './contracts';
import { isUnsaved, ProfileSession } from './profile-session';
import type { Invoke } from './profile-session';
import { upgradeKeypadProfile } from './keypad-model';
import { browserProfile } from './ui-model';

interface Call { command: string; args?: Record<string, unknown>; resolve(value: unknown): void; reject(reason: unknown): void }

function harness(desktop = true) {
  const calls: Call[] = [];
  const invoke: Invoke = <T,>(command: string, args?: Record<string, unknown>) => new Promise<T>((resolve, reject) => {
    calls.push({ command, args, resolve: resolve as (value: unknown) => void, reject });
  });
  // The automatic-save debounce never elapses on its own: tests fire it.
  const scheduled: (() => void)[] = [];
  const schedule = (action: () => void) => {
    scheduled.push(action);
    return () => { const index = scheduled.indexOf(action); if (index >= 0) scheduled.splice(index, 1); };
  };
  const session = new ProfileSession(invoke, browserProfile, desktop, null, Date.now, schedule);
  // Takes the oldest pending call for a command.
  const take = (command: string) => {
    const index = calls.findIndex(call => call.command === command);
    if (index < 0) throw new Error(`no pending ${command} call`);
    return calls.splice(index, 1)[0];
  };
  return { session, calls, take, scheduled, fireDebounce: () => scheduled.splice(0).forEach(run => run()) };
}

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

function stored(id: string, name: string, modified = 1000): StoredProfile {
  return { id, modified_at_ms: modified, profile: upgradeKeypadProfile({ schema_version: 1, name, bindings: [] }) };
}
const summary = (item: StoredProfile) => ({ id: item.id, name: item.profile.name, binding_count: 0, modified_at_ms: item.modified_at_ms, dry_run_only: false });
const listing = (...items: StoredProfile[]): ProfileListing => ({ profiles: items.map(summary), unreadable: [] });
const bootstrapValue = (item: StoredProfile | null, extra: Partial<AppBootstrap> = {}): AppBootstrap =>
  ({ profile: item, profiles: item ? [summary(item)] : [], unreadable: [], storage_path: '/store/profiles', desktop: true, ...extra });

async function ready(profile = stored('a', 'Alpha')) {
  const test = harness();
  const started = test.session.bootstrap();
  test.take('bootstrap').resolve(bootstrapValue(profile));
  await started;
  return test;
}

describe('bootstrap', () => {
  it('starts clean so the close guard does not fire while opening', () => {
    const { session } = harness();
    expect(session.getState().busy).toBe('Opening profiles');
    expect(session.unsaved).toBe(false);
  });

  it('keeps the failure visible with the storage folder and recovers on retry', async () => {
    const { session, take } = harness();
    let run = session.bootstrap();
    take('bootstrap').reject({ message: 'profile storage must be a directory', storage_path: '/Users/me/Library/Application Support/dev.cooltek.openstreamhub/profiles' });
    await run;
    expect(session.getState().bootstrapError).toEqual({ message: 'profile storage must be a directory', storagePath: '/Users/me/Library/Application Support/dev.cooltek.openstreamhub/profiles' });
    expect(session.getState().ready).toBe(false);
    expect(session.getState().busy).toBeNull();

    run = session.bootstrap();
    expect(session.getState().bootstrapError).toBeNull();
    take('bootstrap').resolve(bootstrapValue(stored('a', 'Alpha')));
    await run;
    expect(session.getState().ready).toBe(true);
    expect(session.getState().draft.profileId).toBe('a');
  });

  it('accepts plain string errors', async () => {
    const { session, take } = harness();
    const run = session.bootstrap();
    take('bootstrap').reject('lock is unavailable');
    await run;
    expect(session.getState().bootstrapError).toEqual({ message: 'lock is unavailable', storagePath: null });
  });

  it('discards a superseded bootstrap response', async () => {
    const { session, take } = harness();
    const first = session.bootstrap();
    const second = session.bootstrap();
    const firstCall = take('bootstrap');
    take('bootstrap').resolve(bootstrapValue(stored('b', 'Beta')));
    await second;
    firstCall.resolve(bootstrapValue(stored('a', 'Alpha')));
    await first;
    expect(session.getState().draft.profileId).toBe('b');
  });

  it('opens a new draft and lists unreadable profiles when none can be read', async () => {
    const { session, take } = harness();
    const run = session.bootstrap();
    take('bootstrap').resolve(bootstrapValue(null, { unreadable: [{ id: 'bad', error: 'invalid profile JSON' }] }));
    await run;
    const state = session.getState();
    expect(state.ready).toBe(true);
    expect(state.draft.profileId).toBeNull();
    expect(state.unreadable).toEqual([{ id: 'bad', error: 'invalid profile JSON' }]);
    expect(state.notice).toContain('Unreadable files are kept unchanged');
  });
});

describe('unsaved-draft guard', () => {
  it('runs immediately for a clean draft', async () => {
    const { session } = await ready();
    let runs = 0;
    session.guard('switch profiles', () => { runs += 1; });
    expect(runs).toBe(1);
    expect(session.getState().pendingIntent).toBeNull();
  });

  it('saves the edit first when automatic saving is on, then runs', async () => {
    const { session, take } = await ready();
    let runs = 0;
    session.update(profile => ({ ...profile, name: 'Edited' }));
    session.guard('switch profiles', () => { runs += 1; });
    expect(runs).toBe(0);
    expect(session.getState().pendingIntent).toBeNull();
    const save = take('save_profile');
    expect(save.args).toEqual({ request: expect.objectContaining({ id: 'a', sourceProfileId: 'a' }) });
    save.resolve(stored('a', 'Edited', 2000));
    await flush();
    take('list_profiles').resolve(listing(stored('a', 'Edited', 2000)));
    await flush();
    expect(runs).toBe(1);
    expect(session.unsaved).toBe(false);
  });

  it('asks before discarding when automatic saving is off', async () => {
    const { session } = await ready();
    session.setAutosave(false);
    let runs = 0;
    session.guard('switch profiles', () => { runs += 1; });
    expect(runs).toBe(1);
    expect(session.getState().pendingIntent).toBeNull();

    session.update(profile => ({ ...profile, name: 'Edited' }));
    session.guard('switch profiles', () => { runs += 1; });
    expect(runs).toBe(1);
    expect(session.getState().pendingIntent?.label).toBe('switch profiles');
    session.resolveIntent(false);
    expect(runs).toBe(1);
    expect(session.getState().pendingIntent).toBeNull();

    session.guard('switch profiles', () => { runs += 1; });
    session.resolveIntent(true);
    expect(runs).toBe(2);
  });

  it('asks before discarding when the edit cannot be saved (an invalid name mid-typing)', async () => {
    const { session, scheduled } = await ready();
    session.update(profile => ({ ...profile, name: '' }));
    session.guard('switch profiles', () => {});
    expect(session.getState().pendingIntent?.label).toBe('switch profiles');
    expect(scheduled).toHaveLength(0);
  });

  it('treats a reordered but equal profile as saved', async () => {
    const { session } = await ready();
    const { name, schema_version, bindings, keypad } = session.getState().draft.profile;
    session.edit({ keypad, bindings, schema_version, name } as Profile);
    expect(session.unsaved).toBe(false);
    session.edit({ keypad, bindings, schema_version, name: `${name}!` });
    expect(session.unsaved).toBe(true);
  });
});

describe('window close request', () => {
  it('allows a clean close and waits while busy', async () => {
    const { session, take } = await ready();
    let destroyed = 0;
    expect(session.requestClose(() => { destroyed += 1; })).toBe(true);

    const saving = session.save();
    expect(session.requestClose(() => { destroyed += 1; })).toBe(false);
    expect(session.getState().error).toContain('Wait for the profile operation');
    take('save_profile').reject('disk full');
    await saving;
    expect(destroyed).toBe(0);
  });

  it('saves an unsaved edit and then closes when automatic saving is on', async () => {
    const { session, take } = await ready();
    let destroyed = 0;
    session.update(profile => ({ ...profile, name: 'Edited' }));
    expect(session.requestClose(() => { destroyed += 1; })).toBe(false);
    const save = take('save_profile');
    save.resolve(stored('a', 'Edited', 2000));
    await flush();
    take('list_profiles').resolve(listing(stored('a', 'Edited', 2000)));
    await flush();
    expect(destroyed).toBe(1);
    expect(session.getState().pendingIntent).toBeNull();
  });

  it('hands over to the confirmation when the automatic save fails', async () => {
    const { session, take } = await ready();
    let destroyed = 0;
    session.update(profile => ({ ...profile, name: 'Edited' }));
    expect(session.requestClose(() => { destroyed += 1; })).toBe(false);
    take('save_profile').reject('disk full');
    await flush();
    expect(destroyed).toBe(0);
    expect(session.getState().pendingIntent?.label).toBe('close the window');
    session.resolveIntent(true);
    expect(destroyed).toBe(1);
  });

  it('confirms an unsaved close when automatic saving is off', async () => {
    const { session } = await ready();
    session.setAutosave(false);
    let destroyed = 0;
    session.update(profile => ({ ...profile, name: 'Edited' }));
    expect(session.requestClose(() => { destroyed += 1; })).toBe(false);
    expect(session.getState().pendingIntent?.label).toBe('close the window');
    session.resolveIntent(true);
    expect(destroyed).toBe(1);
  });
});

describe('automatic saving', () => {
  it('saves a settled edit to the saved profile without a manual save', async () => {
    const { session, scheduled, take, fireDebounce } = await ready();
    session.update(profile => ({ ...profile, name: 'Edited' }));
    expect(session.unsaved).toBe(true);
    expect(scheduled).toHaveLength(1);
    fireDebounce();
    const save = take('save_profile');
    expect(save.args).toEqual({ request: expect.objectContaining({ id: 'a', expectedModifiedAtMs: 1000, sourceProfileId: 'a' }) });
    save.resolve(stored('a', 'Edited', 2000));
    await flush();
    expect(session.unsaved).toBe(false);
    expect(scheduled).toHaveLength(0);
    expect(session.getState().draft.modifiedAtMs).toBe(2000);
    // The recovery copy is gone once the file holds the edit.
    expect(session.getState().restorePrompt).toBeNull();
  });

  it('does not save a new profile, an invalid draft, and cancels with the setting off', async () => {
    const { session, scheduled } = await ready();
    // A new profile has no file to save to.
    session.newProfile();
    session.update(profile => ({ ...profile, name: 'Idea' }));
    expect(scheduled).toHaveLength(0);
    expect(session.unsaved).toBe(true);

    // An invalid name is never scheduled; fixing it schedules the save.
    const valid = await ready();
    valid.session.update(profile => ({ ...profile, name: '' }));
    expect(valid.scheduled).toHaveLength(0);
    valid.session.update(profile => ({ ...profile, name: 'Fixed' }));
    expect(valid.scheduled).toHaveLength(1);

    // The setting cancels a pending save and restores it when on again.
    const gated = await ready();
    gated.session.update(profile => ({ ...profile, name: 'Edited' }));
    expect(gated.scheduled).toHaveLength(1);
    gated.session.setAutosave(false);
    expect(gated.scheduled).toHaveLength(0);
    gated.session.setAutosave(true);
    expect(gated.scheduled).toHaveLength(1);
  });

  it('waits for a running profile operation and saves afterwards', async () => {
    const { session, calls, scheduled, take, fireDebounce } = await ready();
    const manual = session.save();
    session.update(profile => ({ ...profile, name: 'Edited' }));
    fireDebounce();
    // The manual save is still the only save request; the automatic one
    // waits for its turn.
    expect(calls.filter(call => call.command === 'save_profile')).toHaveLength(1);
    expect(scheduled).toHaveLength(1);
    take('save_profile').resolve(stored('a', 'Alpha', 2000));
    await Promise.resolve(); await new Promise(resolve => setTimeout(resolve, 0));
    take('list_profiles').resolve(listing(stored('a', 'Alpha')));
    await manual;
    // The manual save left the draft clean; nothing else stays pending.
    expect(scheduled).toHaveLength(0);
    expect(session.unsaved).toBe(false);
  });

  it('stops after a disk conflict and recovers on a reload', async () => {
    const { session, scheduled, take, fireDebounce } = await ready(stored('a', 'Alpha', 1234));
    session.update(profile => ({ ...profile, name: 'Edited' }));
    fireDebounce();
    take('save_profile').reject('this profile changed on disk after it was loaded, possibly in another OpenStreamHub window; reload the saved profile before saving again');
    await flush();
    expect(session.getState().autosaveBlocked).toBe(true);
    expect(session.getState().error).toContain('changed on disk');
    expect(session.unsaved).toBe(true);
    // Further edits do not retry the save.
    session.update(profile => ({ ...profile, name: 'Edited twice' }));
    expect(scheduled).toHaveLength(0);
    // Reloading the saved profile clears the stop.
    const loading = session.load('a');
    take('load_profile').resolve(stored('a', 'Alpha', 5000));
    await loading;
    expect(session.getState().autosaveBlocked).toBe(false);
    session.update(profile => ({ ...profile, name: 'After reload' }));
    expect(scheduled).toHaveLength(1);
  });

  it('keeps the undo history across an automatic save', async () => {
    const { session, scheduled, take, fireDebounce } = await ready();
    session.update(profile => ({ ...profile, name: 'Edited' }));
    fireDebounce();
    take('save_profile').resolve(stored('a', 'Edited', 2000));
    await flush();
    take('list_profiles').resolve(listing(stored('a', 'Edited', 2000)));
    await flush();
    expect(session.getState().canUndo).toBe(true);
    expect(session.undo()).toBe(true);
    expect(session.getState().draft.profile.name).toBe('Alpha');
    // The undone draft differs from the saved one, so it is saved too.
    expect(scheduled).toHaveLength(1);
    expect(session.unsaved).toBe(true);
  });

  it('skips a superseded response and keeps the newer edit pending', async () => {
    const { session, scheduled, take, fireDebounce } = await ready();
    session.update(profile => ({ ...profile, name: 'First' }));
    fireDebounce();
    const first = take('save_profile');
    // A second edit lands while the request is out; it is not lost.
    session.update(profile => ({ ...profile, name: 'Second' }));
    first.resolve(stored('a', 'First', 2000));
    await flush();
    expect(scheduled).toHaveLength(1);
    expect(session.unsaved).toBe(true);
    fireDebounce();
    const second = take('save_profile');
    expect(second.args).toEqual({ request: expect.objectContaining({ id: 'a' }) });
    second.resolve(stored('a', 'Second', 3000));
    await flush();
    expect(session.unsaved).toBe(false);
    expect(session.getState().draft.modifiedAtMs).toBe(3000);
  });
});

describe('earlier versions', () => {
  it('lists the kept versions of a profile', async () => {
    const { session, take } = await ready();
    const listing = session.loadVersions('a');
    const call = take('profile_versions');
    expect(call.args).toEqual({ id: 'a' });
    call.resolve([{ modified_at_ms: 500 }, { modified_at_ms: 200 }]);
    await listing;
    expect(session.getState().versions).toEqual([{ modified_at_ms: 500 }, { modified_at_ms: 200 }]);
    expect(session.getState().versionsId).toBe('a');
    expect(session.getState().versionsError).toBeNull();
  });

  it('reports a failed listing instead of showing an empty list', async () => {
    const { session, take } = await ready();
    const listing = session.loadVersions('a');
    take('profile_versions').reject('gone');
    await listing;
    expect(session.getState().versions).toBeNull();
    expect(session.getState().versionsError).toContain('gone');
  });

  it('restores a version through an ordinary save, keeping the open profile', async () => {
    const { session, take } = await ready(stored('a', 'Alpha', 1234));
    const restore = session.restoreVersion(500);
    const load = take('load_profile_version');
    expect(load.args).toEqual({ id: 'a', modifiedAtMs: 500 });
    load.resolve({ id: 'a', modified_at_ms: 500, profile: upgradeKeypadProfile({ schema_version: 1, name: 'Earlier', bindings: [] }) });
    await Promise.resolve();
    const save = take('save_profile');
    expect(save.args).toEqual({ request: expect.objectContaining({ id: 'a', expectedModifiedAtMs: 1234, sourceProfileId: 'a' }) });
    save.resolve(stored('a', 'Earlier', 3000));
    await Promise.resolve(); await new Promise(resolve => setTimeout(resolve, 0));
    take('list_profiles').resolve(listing(stored('a', 'Earlier')));
    await restore;
    const state = session.getState();
    expect(state.draft.profile.name).toBe('Earlier');
    expect(state.draft.modifiedAtMs).toBe(3000);
    expect(state.notice).toContain('Restored the version from');
    expect(state.versions).toBeNull();
    expect(state.versionsId).toBeNull();
    expect(session.unsaved).toBe(false);
    expect(session.getState().canUndo).toBe(false);
  });

  it('keeps the draft when the restore save conflicts', async () => {
    const { session, take } = await ready(stored('a', 'Alpha', 1234));
    const restore = session.restoreVersion(500);
    const load = take('load_profile_version');
    load.resolve({ id: 'a', modified_at_ms: 500, profile: upgradeKeypadProfile({ schema_version: 1, name: 'Earlier', bindings: [] }) });
    await Promise.resolve();
    take('save_profile').reject('this profile changed on disk after it was loaded, possibly in another OpenStreamHub window; reload the saved profile before saving again');
    await restore;
    expect(session.getState().error).toContain('Could not restore the version');
    expect(session.getState().draft.profile.name).toBe('Alpha');
    expect(session.unsaved).toBe(false);
  });

  it('does nothing without an open saved profile', async () => {
    const { session } = await ready();
    session.newProfile();
    await session.restoreVersion(500);
    expect(session.getState().busy).toBeNull();
    expect(session.getState().error).toBeNull();
  });
});

describe('stale responses', () => {
  it('applies only the latest profile load', async () => {
    const { session, take } = await ready();
    const first = session.load('b');
    const second = session.load('c');
    const firstCall = take('load_profile');
    take('load_profile').resolve(stored('c', 'Gamma'));
    await second;
    firstCall.resolve(stored('b', 'Beta'));
    await first;
    expect(session.getState().draft.profileId).toBe('c');
    expect(session.getState().busy).toBeNull();
    expect(session.getState().notice).toBe('Loaded Gamma.');
  });

  it('ignores a failure from a superseded load', async () => {
    const { session, take } = await ready();
    const first = session.load('b');
    const second = session.load('c');
    const firstCall = take('load_profile');
    take('load_profile').resolve(stored('c', 'Gamma'));
    await second;
    firstCall.reject('gone');
    await first;
    expect(session.getState().error).toBeNull();
  });

  it('drops a profile list that a later load or edit superseded', async () => {
    const { session, take } = await ready();
    const refresh = session.refreshProfiles();
    const load = session.load('b');
    take('load_profile').resolve(stored('b', 'Beta'));
    await load;
    take('list_profiles').resolve(listing(stored('x', 'Stale')));
    await refresh;
    expect(session.getState().profiles.map(item => item.id)).toEqual(['a']);

    const again = session.refreshProfiles();
    session.update(profile => ({ ...profile, name: 'Edited' }));
    take('list_profiles').reject('io error');
    await again;
    expect(session.getState().error).toBeNull();
  });

  it('applies a current profile list including unreadable entries', async () => {
    const { session, take } = await ready();
    const refresh = session.refreshProfiles();
    take('list_profiles').resolve({ ...listing(stored('a', 'Alpha'), stored('b', 'Beta')), unreadable: [{ id: 'bad', error: 'corrupt' }] });
    await refresh;
    expect(session.getState().profiles).toHaveLength(2);
    expect(session.getState().unreadable).toEqual([{ id: 'bad', error: 'corrupt' }]);
  });

  it('discards binding previews after an edit or a selection change', async () => {
    const { session, take } = await ready();
    const event: ReplayEvent = { timestamp_us: 0, module: 'm003k', control: 0, gesture: 'pressed', serial_number: null, action: { type: 'toggle_mute' } };
    let preview = session.previewBinding({ module: 'm003k', control: 0, gesture: 'pressed', serial: '', faderPosition: 50 });
    session.update(profile => ({ ...profile, name: 'Edited' }));
    take('preview_binding').resolve(event);
    await preview;
    expect(session.getState().preview).toEqual({ status: 'idle' });

    preview = session.previewBinding({ module: 'm003k', control: 0, gesture: 'pressed', serial: '', faderPosition: 50 });
    session.clearPreview();
    take('preview_binding').reject('late failure');
    await preview;
    expect(session.getState().preview).toEqual({ status: 'idle' });

    preview = session.previewBinding({ module: 'm005s', control: 0, gesture: 'position_changed', serial: 'Desk', faderPosition: 37 });
    const call = take('preview_binding');
    expect(call.args).toMatchObject({ serialNumber: 'Desk', value: { type: 'position', raw: 37, maximum: 100 } });
    call.resolve(event);
    await preview;
    expect(session.getState().preview).toEqual({ status: 'completed', event });
  });

  it('returns keypad previews only while they are current', async () => {
    const { session, take } = await ready();
    const result: KeypadPreviewResult = { path: ['root'], depth: 0, events: [], planned_action_count: 0 };
    const stale = session.previewKeypad({ path: ['root'], control: 0, serial: '' });
    const current = session.previewKeypad({ path: ['root'], control: 1, serial: '' });
    const staleCall = take('preview_keypad');
    take('preview_keypad').resolve(result);
    expect(await current).toEqual(result);
    staleCall.resolve({ ...result, path: ['root', 'gone'] });
    expect(await stale).toBeNull();
    expect(session.getState().keypadPreview).toEqual({ status: 'completed', result });
  });

  it('rejects a keypad preview path that does not exist in the draft', async () => {
    const { session, take } = await ready();
    const run = session.previewKeypad({ path: ['root'], control: 0, serial: '' });
    take('preview_keypad').resolve({ path: ['root', 'missing'], depth: 1, events: [], planned_action_count: 0 });
    expect(await run).toBeNull();
    expect(session.getState().keypadPreview.status).toBe('failed');
  });

  it('discards replay and validation results computed from an older draft', async () => {
    const { session, take } = await ready();
    const replay = session.replayCapture('knobs');
    const validation = session.validate();
    session.update(profile => ({ ...profile, name: 'Edited' }));
    take('replay_demo').resolve({ events: [] } as unknown as ReplayResult);
    take('validate_profile').reject('invalid');
    await Promise.all([replay, validation]);
    expect(session.getState().replay).toEqual({ status: 'idle' });
    expect(session.getState().validation).toBe('idle');
    expect(session.getState().error).toBeNull();
  });
});

describe('saving and deleting', () => {
  it('sends the loaded modification time and keeps the draft on a conflict', async () => {
    const { session, take } = await ready(stored('a', 'Alpha', 1234));
    session.update(profile => ({ ...profile, name: 'Mine' }));
    const save = session.save();
    const call = take('save_profile');
    expect(call.args).toEqual({ request: expect.objectContaining({ id: 'a', expectedModifiedAtMs: 1234, sourceProfileId: 'a' }) });
    call.reject('this profile changed on disk after it was loaded, possibly in another OpenStreamHub window; reload the saved profile before saving again');
    await save;
    const state = session.getState();
    expect(state.error).toContain('changed on disk');
    expect(state.draft.profile.name).toBe('Mine');
    expect(isUnsaved(state.draft)).toBe(true);
    expect(state.busy).toBeNull();
  });

  it('creates new profiles without an expected time and refreshes the list after saving', async () => {
    const { session, take } = await ready();
    session.newProfile();
    const save = session.save();
    const call = take('save_profile');
    expect(call.args).toEqual({ request: expect.objectContaining({ id: null, expectedModifiedAtMs: null, sourceProfileId: null }) });
    const created = stored('n', 'New profile', 2000);
    call.resolve(created);
    await flush();
    take('list_profiles').resolve(listing(created, stored('a', 'Alpha')));
    await save;
    const state = session.getState();
    expect(state.draft.profileId).toBe('n');
    expect(state.draft.modifiedAtMs).toBe(2000);
    expect(state.profiles.map(item => item.id)).toEqual(['n', 'a']);
    expect(session.unsaved).toBe(false);
  });

  it('keeps a deleted open profile as unsaved changes', async () => {
    const alpha = stored('a', 'Alpha', 1234);
    const { session, take } = await ready(alpha);
    const remove = session.deleteProfile(summary(alpha));
    const call = take('delete_profile');
    expect(call.args).toEqual({ id: 'a', expectedModifiedAtMs: 1234 });
    call.resolve(null);
    await flush();
    take('list_profiles').resolve(listing());
    await remove;
    const state = session.getState();
    expect(state.draft.profileId).toBeNull();
    expect(state.draft.profile.name).toBe('Alpha');
    expect(session.unsaved).toBe(true);
    expect(state.profiles).toEqual([]);
    expect(state.notice).toContain('unsaved changes');
  });

  it('reports a failed delete without changing the draft', async () => {
    const alpha = stored('a', 'Alpha');
    const { session, take } = await ready(alpha);
    const remove = session.deleteProfile(summary(alpha));
    take('delete_profile').reject('conflict');
    await remove;
    expect(session.getState().error).toBe('Could not delete profile: conflict');
    expect(session.getState().draft.profileId).toBe('a');
  });
});

describe('notice actions', () => {
  it('carries an action with its notice and drops it when the notice changes', () => {
    const { session } = harness(false);
    let undone = 0;
    session.setNotice('Replaced Timer with Open URL on Knob 1.', { label: 'Undo', run: () => { undone += 1; } });
    expect(session.getState().notice).toContain('Replaced Timer');
    expect(session.getState().noticeAction).toMatchObject({ label: 'Undo' });
    session.setNotice('Saved Alpha.');
    expect(session.getState().noticeAction).toBeNull();
    expect(undone).toBe(0);
  });

  it('keeps a fresh action when a notice write replaces both', () => {
    const { session } = harness(false);
    session.setNotice('First.', { label: 'Undo', run: () => {} });
    session.setNotice('Second.', { label: 'Redo', run: () => {} });
    expect(session.getState().noticeAction?.label).toBe('Redo');
    // Clearing the notice clears its action.
    session.setNotice(null);
    expect(session.getState().notice).toBeNull();
    expect(session.getState().noticeAction).toBeNull();
  });
});
