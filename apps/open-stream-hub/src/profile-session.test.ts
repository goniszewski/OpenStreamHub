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
  const session = new ProfileSession(invoke, browserProfile, desktop);
  // Takes the oldest pending call for a command.
  const take = (command: string) => {
    const index = calls.findIndex(call => call.command === command);
    if (index < 0) throw new Error(`no pending ${command} call`);
    return calls.splice(index, 1)[0];
  };
  return { session, calls, take };
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
  it('runs immediately for a clean draft and asks before discarding edits', async () => {
    const { session } = await ready();
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
  it('allows a clean close, confirms an unsaved close and waits while busy', async () => {
    const { session, take } = await ready();
    let destroyed = 0;
    expect(session.requestClose(() => { destroyed += 1; })).toBe(true);

    session.update(profile => ({ ...profile, name: 'Edited' }));
    expect(session.requestClose(() => { destroyed += 1; })).toBe(false);
    expect(session.getState().pendingIntent?.label).toBe('close the window');
    session.resolveIntent(true);
    expect(destroyed).toBe(1);

    const saving = session.save();
    expect(session.requestClose(() => { destroyed += 1; })).toBe(false);
    expect(session.getState().error).toContain('Wait for the profile operation');
    expect(session.getState().pendingIntent).toBeNull();
    take('save_profile').reject('disk full');
    await saving;
    expect(destroyed).toBe(1);
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

  it('keeps a deleted open profile as an unsaved draft', async () => {
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
    expect(state.notice).toContain('unsaved draft');
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
