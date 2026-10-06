import { describe, expect, it } from 'vitest';
import type { AppBootstrap, Profile, ProfileListing, StoredProfile } from './contracts';
import { DraftPersister, DraftStore, newDraftKey } from './draft-store';
import { FakeStorage } from './fake-storage';
import { createKeypadFolder, keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import type { Invoke, SessionState } from './profile-session';
import { browserProfile } from './ui-model';
import { syncKeypad, usablePath } from './useDeviceEditor';
import { historyLimit } from './edit-history';

interface Call { command: string; args?: Record<string, unknown>; resolve(value: unknown): void; reject(reason: unknown): void }

function harness(profile: Profile = browserProfile) {
  const calls: Call[] = [];
  const invoke: Invoke = <T,>(command: string, args?: Record<string, unknown>) => new Promise<T>((resolve, reject) => {
    calls.push({ command, args, resolve: resolve as (value: unknown) => void, reject });
  });
  const storage = new FakeStorage();
  const scheduled: (() => void)[] = [];
  const persister = new DraftPersister(new DraftStore(storage), action => { scheduled.push(action); return () => {}; });
  let clock = 1000;
  const session = new ProfileSession(invoke, profile, true, persister, () => clock);
  const take = (command: string) => {
    const index = calls.findIndex(call => call.command === command);
    if (index < 0) throw new Error(`no pending ${command} call`);
    return calls.splice(index, 1)[0];
  };
  return { session, storage, take, advance: (ms: number) => { clock += ms; }, persister };
}

const stored = (id: string, profile: Profile, modified = 1000): StoredProfile => ({ id, modified_at_ms: modified, profile: upgradeKeypadProfile(profile) });
const named = (name: string): Profile => ({ schema_version: 1, name, bindings: [] });
const summary = (item: StoredProfile) => ({ id: item.id, name: item.profile.name, binding_count: 0, modified_at_ms: item.modified_at_ms });
const listing = (...items: StoredProfile[]): ProfileListing => ({ profiles: items.map(summary), unreadable: [] });

async function opened(profile: Profile = named('Alpha')) {
  const test = harness();
  const started = test.session.bootstrap();
  const item = stored('a', profile);
  test.take('bootstrap').resolve({ profile: item, profiles: [summary(item)], unreadable: [], storage_path: '/s', desktop: true } satisfies AppBootstrap);
  await started;
  return { ...test, item };
}
const rename = (session: ProfileSession, name: string, key?: string) => session.update(profile => ({ ...profile, name }), key);
const nameOf = (session: ProfileSession) => session.getState().draft.profile.name;

describe('undo and redo', () => {
  it('walks back and forward through draft edits and updates the flags', async () => {
    const { session } = await opened();
    expect(session.getState()).toMatchObject({ canUndo: false, canRedo: false });
    rename(session, 'B'); rename(session, 'C');
    expect(session.getState()).toMatchObject({ canUndo: true, canRedo: false });
    expect(session.undo()).toBe(true);
    expect(nameOf(session)).toBe('B');
    expect(session.getState()).toMatchObject({ canUndo: true, canRedo: true });
    session.undo();
    expect(nameOf(session)).toBe('Alpha');
    expect(session.getState()).toMatchObject({ canUndo: false, canRedo: true });
    expect(session.undo()).toBe(false);
    session.redo(); session.redo();
    expect(nameOf(session)).toBe('C');
    expect(session.redo()).toBe(false);
  });

  it('becomes clean again when undoing back to the saved profile, and dirty on redo', async () => {
    const { session } = await opened();
    rename(session, 'B');
    expect(session.unsaved).toBe(true);
    session.undo();
    expect(session.unsaved).toBe(false);
    session.redo();
    expect(session.unsaved).toBe(true);
  });

  it('clears the redo stack on a new edit', async () => {
    const { session } = await opened();
    rename(session, 'B'); rename(session, 'C');
    session.undo();
    rename(session, 'D');
    expect(session.getState().canRedo).toBe(false);
    expect(session.redo()).toBe(false);
    expect(nameOf(session)).toBe('D');
  });

  it('coalesces rapid keyed edits and splits them after a pause or a different field', async () => {
    const { session, advance } = await opened();
    for (const text of ['B', 'Bo', 'Bob']) { rename(session, text, 'name'); advance(100); }
    session.undo();
    expect(nameOf(session)).toBe('Alpha');
    session.redo();
    advance(1000);
    rename(session, 'Bobby', 'name');
    session.undo();
    expect(nameOf(session)).toBe('Bob');
    // Edits without a key never coalesce.
    advance(10); rename(session, 'x1'); advance(10); rename(session, 'x2');
    session.undo();
    expect(nameOf(session)).toBe('x1');
    // A different key does not merge either.
    advance(10); rename(session, 'y1', 'a'); advance(10); rename(session, 'y2', 'b');
    session.undo();
    expect(nameOf(session)).toBe('y1');
  });

  it('does not merge a keyed edit into a burst that was undone', async () => {
    const { session, advance } = await opened();
    rename(session, 'B', 'name'); advance(50);
    session.undo();
    advance(50);
    rename(session, 'C', 'name');
    session.undo();
    expect(nameOf(session)).toBe('Alpha');
  });

  it('keeps at most the newest 100 steps', async () => {
    const { session } = await opened();
    for (let index = 1; index <= historyLimit + 10; index += 1) rename(session, `n${index}`);
    let steps = 0;
    while (session.undo()) steps += 1;
    expect(steps).toBe(historyLimit);
    expect(nameOf(session)).toBe('n10');
  });

  it('does nothing while a profile operation is running', async () => {
    const { session, take } = await opened();
    rename(session, 'B');
    const saving = session.save();
    expect(session.undo()).toBe(false);
    expect(nameOf(session)).toBe('B');
    take('save_profile').reject('nope');
    await saving;
  });

  it('never calls the service or changes the saved baseline', async () => {
    const { session, item } = await opened();
    const before = session.getState().draft.savedSnapshot;
    rename(session, 'B'); session.undo(); session.redo();
    expect(session.getState().draft.savedSnapshot).toBe(before);
    expect(session.getState().draft.modifiedAtMs).toBe(item.modified_at_ms);
  });
});

describe('history lifetime', () => {
  it('is cleared by a successful save', async () => {
    const { session, take, item } = await opened();
    rename(session, 'B');
    const saving = session.save();
    take('save_profile').resolve(stored('a', named('B'), 2000));
    await Promise.resolve(); await new Promise(resolve => setTimeout(resolve, 0));
    take('list_profiles').resolve(listing(item));
    await saving;
    expect(session.getState()).toMatchObject({ canUndo: false, canRedo: false });
    expect(session.unsaved).toBe(false);
    expect(session.undo()).toBe(false);
  });

  it('is cleared by switching profiles, new profile and restore decisions', async () => {
    const { session, take } = await opened();
    rename(session, 'B');
    const loading = session.load('z');
    take('load_profile').resolve(stored('z', named('Zed')));
    await loading;
    expect(session.getState()).toMatchObject({ canUndo: false, canRedo: false });
    rename(session, 'Q');
    session.newProfile();
    expect(session.getState()).toMatchObject({ canUndo: false, canRedo: false });
  });

  it('is cleared by duplicate and import, which switch to a new saved profile', async () => {
    const { session, take } = await opened();
    rename(session, 'B');
    const duplicating = session.resolveDuplicate('draft');
    await Promise.resolve();
    take('list_profiles').resolve(listing());
    await new Promise(resolve => setTimeout(resolve, 0));
    take('save_profile').resolve(stored('b', named('B copy')));
    await new Promise(resolve => setTimeout(resolve, 0));
    take('list_profiles').resolve(listing());
    await duplicating;
    expect(session.getState().draft.profileId).toBe('b');
    expect(session.getState()).toMatchObject({ canUndo: false, canRedo: false });
  });

  it('is cleared when restoring a recovered draft', async () => {
    const test = harness();
    test.storage.setItem('openstreamhub.draft.a', JSON.stringify({ profile: upgradeKeypadProfile(named('Recovered')), base_modified_at_ms: 1000, saved_at: 5 }));
    const started = test.session.bootstrap();
    const item = stored('a', named('Alpha'));
    test.take('bootstrap').resolve({ profile: item, profiles: [summary(item)], unreadable: [], storage_path: '/s', desktop: true } satisfies AppBootstrap);
    await started;
    expect(test.session.getState().restorePrompt).not.toBeNull();
    test.session.resolveRestore(true);
    expect(nameOf(test.session)).toBe('Recovered');
    expect(test.session.getState()).toMatchObject({ canUndo: false, canRedo: false });
  });
});

describe('draft persistence follows undo and redo', () => {
  const stash = (storage: ReturnType<typeof harness>['storage'], key = 'a') => {
    const raw = storage.getItem(`openstreamhub.draft.${key}`);
    return raw ? (JSON.parse(raw).profile.name as string) : null;
  };

  it('writes the undone draft and clears the copy when the draft is clean again', async () => {
    const { session, storage } = await opened();
    rename(session, 'B'); rename(session, 'C');
    session.flushDraft();
    expect(stash(storage)).toBe('C');
    session.undo();
    session.flushDraft();
    expect(stash(storage)).toBe('B');
    session.undo();
    session.flushDraft();
    expect(stash(storage)).toBeNull();
    session.redo();
    session.flushDraft();
    expect(stash(storage)).toBe('B');
  });

  it('keeps a never-saved draft key out of storage once undone to empty', async () => {
    const { session, storage, take } = harness();
    const started = session.bootstrap();
    take('bootstrap').resolve({ profile: null, profiles: [], unreadable: [], storage_path: '/s', desktop: true } satisfies AppBootstrap);
    await started;
    rename(session, 'Something');
    session.flushDraft();
    expect(storage.getItem(`openstreamhub.draft.${newDraftKey}`)).not.toBeNull();
    session.undo();
    session.flushDraft();
    expect(storage.getItem(`openstreamhub.draft.${newDraftKey}`)).toBeNull();
  });
});

describe('keypad folder removed by undo', () => {
  it('falls back to the root page instead of an open folder that no longer exists', async () => {
    const { session, take } = harness();
    const started = session.bootstrap();
    take('bootstrap').resolve({ profile: null, profiles: [], unreadable: [], storage_path: '/s', desktop: true } satisfies AppBootstrap);
    await started;
    const folder = 'page-1';
    session.edit(createKeypadFolder(session.getState().draft.profile, [keypadRoot], 3, folder));
    const keypad = { generation: session.getState().draftGeneration, path: [keypadRoot, folder] };
    const state = (): SessionState => session.getState();
    expect(usablePath(state(), keypad)).toEqual([keypadRoot, folder]);
    session.undo();
    expect(usablePath(state(), keypad)).toEqual([keypadRoot]);
    session.redo();
    expect(usablePath(state(), keypad)).toEqual([keypadRoot, folder]);
  });

  it('resets the stored path once the folder is gone, so redo stays on the root page', async () => {
    const { session, take } = harness();
    const started = session.bootstrap();
    take('bootstrap').resolve({ profile: null, profiles: [], unreadable: [], storage_path: '/s', desktop: true } satisfies AppBootstrap);
    await started;
    const folder = 'page-1';
    session.edit(createKeypadFolder(session.getState().draft.profile, [keypadRoot], 3, folder));
    let keypad = { generation: session.getState().draftGeneration, path: [keypadRoot, folder] };
    expect(syncKeypad(session.getState(), keypad)).toBe(keypad);
    session.undo();
    keypad = syncKeypad(session.getState(), keypad);
    expect(keypad.path).toEqual([keypadRoot]);
    expect(syncKeypad(session.getState(), keypad)).toBe(keypad);
    session.redo();
    expect(syncKeypad(session.getState(), keypad).path).toEqual([keypadRoot]);
  });
});
