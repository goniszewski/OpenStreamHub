import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { AppBootstrap, Profile, ProfileListing, StoredProfile } from './contracts';
import { DraftPersister, DraftStore, newDraftKey, restoreDecision } from './draft-store';
import { FakeStorage } from './fake-storage';
import { upgradeKeypadProfile } from './keypad-model';
import { parseImportedProfile } from './profile-library';
import { ProfileSession } from './profile-session';
import type { Invoke } from './profile-session';
import { editBinding } from './ui-model';

// The service crate's own fixture, so the frontend is tested against the real shape.
const fixtureText = readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/dial-functions.json', import.meta.url), 'utf8');
const fixture = JSON.parse(fixtureText) as Profile;
const dialFunctions = () => structuredClone(fixture.dial_functions);

function harness() {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const saved: StoredProfile[] = [];
  const invoke: Invoke = <T,>(command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    let result: unknown = { valid: true };
    if (command === 'list_profiles') result = { profiles: saved.map(item => ({ id: item.id, name: item.profile.name, binding_count: 0, modified_at_ms: 1, dry_run_only: false })), unreadable: [] } satisfies ProfileListing;
    if (command === 'save_profile') {
      const item = { id: `p${saved.length}`, modified_at_ms: 1, profile: structuredClone((args as { request: { profile: Profile } }).request.profile) };
      saved.push(item); result = item;
    }
    if (command === 'bootstrap') {
      const item = { id: 'a', modified_at_ms: 1, profile: upgradeKeypadProfile(structuredClone(fixture)) };
      saved.push(item);
      result = { profile: item, profiles: [{ id: 'a', name: item.profile.name, binding_count: 1, modified_at_ms: 1, dry_run_only: false }], unreadable: [], storage_path: '/s', desktop: true } satisfies AppBootstrap;
    }
    if (command === 'export_profile') result = { path: '/x', file_name: 'x.json' };
    return Promise.resolve(result as T);
  };
  const storage = new FakeStorage();
  const persister = new DraftPersister(new DraftStore(storage), () => () => {});
  const session = new ProfileSession(invoke, fixture, true, persister, () => 1000);
  return { session, calls, saved, storage, persister };
}

async function open() {
  const test = harness();
  await test.session.bootstrap();
  test.calls.length = 0;
  return test;
}
const lastArg = (calls: { command: string; args?: Record<string, unknown> }[], command: string) =>
  ((args => (args.request ?? args) as { profile: Profile })(calls.filter(call => call.command === command).at(-1)!.args as { request?: unknown })).profile;

describe('dial_functions survive every frontend profile path', () => {
  it('types accept the service fixture and keep it through edits, undo/redo, duplicate, export and import', async () => {
    const { session, calls } = await open();
    const expected = dialFunctions();
    const dial = () => session.getState().draft.profile.dial_functions;
    expect(dial()).toEqual(expected);

    session.update(profile => editBinding(profile, { module: 'm003k', control: 1, gesture: 'clockwise', serial: '' }, { type: 'volume', delta: 3 }));
    session.update(profile => ({ ...profile, name: 'Renamed' }), 'profile-name');
    expect(dial()).toEqual(expected);
    session.undo(); session.undo();
    expect(dial()).toEqual(expected);
    session.redo(); session.redo();
    expect(dial()).toEqual(expected);

    await session.exportProfile();
    expect(lastArg(calls, 'export_profile').dial_functions).toEqual(expected);

    session.requestDuplicate();
    await session.resolveDuplicate('draft');
    const copy = lastArg(calls, 'save_profile');
    expect(copy.dial_functions).toEqual(expected);
    expect(JSON.stringify(copy.dial_functions)).toBe(JSON.stringify(expected));

    const imported = parseImportedProfile(JSON.stringify(copy));
    expect(imported.dial_functions).toEqual(expected);
    await session.importProfile(JSON.stringify(copy));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(lastArg(calls, 'validate_profile').dial_functions).toEqual(expected);
    expect(lastArg(calls, 'save_profile').dial_functions).toEqual(expected);
  });

  it('keeps dial_functions when a file with them is imported and parsed', () => {
    expect(parseImportedProfile(fixtureText).dial_functions).toEqual(fixture.dial_functions);
  });

  it('restores a draft with dial_functions intact', () => {
    const storage = new FakeStorage();
    const store = new DraftStore(storage);
    const profile = upgradeKeypadProfile(structuredClone(fixture));
    expect(store.write(newDraftKey, profile, null)).toBe('written');
    const stored = store.read(newDraftKey);
    expect(stored?.profile.dial_functions).toEqual(fixture.dial_functions);
    expect(restoreDecision(stored, null)).toEqual({ kind: 'offer', changedOnDisk: false });
  });
});
