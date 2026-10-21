import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import type { Profile, StoredProfile } from './contracts';
import { DraftPersister, DraftStore, newDraftKey } from './draft-store';
import { FakeStorage } from './fake-storage';
import { upgradeKeypadProfile } from './keypad-model';
import { maximumImportBytes } from './profile-library';
import { ProfileSession } from './profile-session';
import type { Invoke } from './profile-session';
import { copyFallback, copyText, importFile, ProfileTransfer } from './ProfileTransfer';
import { browserProfile } from './ui-model';

const profile = (name: string): Profile => upgradeKeypadProfile({ schema_version: 1, name, bindings: [] });
const flush = () => new Promise(resolve => setTimeout(resolve, 0));

// A small in-memory stand-in for the Rust commands.
function backend(initial: Record<string, StoredProfile>, options: { rejectValidation?: string; exportPath?: string } = {}) {
  const files = new Map(Object.entries(initial));
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  let counter = 0;
  const summaries = () => [...files.values()].map(item => ({ id: item.id, name: item.profile.name, binding_count: 0, modified_at_ms: item.modified_at_ms, dry_run_only: false }));
  const invoke: Invoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ command, args });
    switch (command) {
      case 'bootstrap': return { profile: [...files.values()][0] ?? null, profiles: summaries(), unreadable: [], storage_path: '/s', desktop: true } as T;
      case 'list_profiles': return { profiles: summaries(), unreadable: [] } as T;
      case 'load_profile': return files.get(args!.id as string) as T;
      case 'validate_profile':
        if ((args!.profile as Profile).name === options.rejectValidation) throw 'unknown field `extra`';
        return { valid: true } as T;
      case 'save_profile': {
        const request = args!.request as { id: string | null; profile: Profile };
        const id = request.id ?? `new-${++counter}`;
        const saved = { id, profile: request.profile, modified_at_ms: 2000 + counter };
        files.set(id, saved);
        return saved as T;
      }
      case 'delete_profile': files.delete(args!.id as string); return null as T;
      case 'export_profile': return { path: options.exportPath ?? '/s/exports/x-20261005-120000.json', bytes: 1234 } as T;
      default: throw new Error(`unexpected ${command}`);
    }
  };
  return { files, calls, invoke };
}
const stored = (id: string, name: string, modified = 1000): StoredProfile => ({ id, profile: profile(name), modified_at_ms: modified });
const names = (calls: { command: string; args?: Record<string, unknown> }[], command: string) => calls.filter(call => call.command === command).map(call => ((call.args!.request ?? call.args) as { profile?: Profile }).profile?.name);

async function open(initial: StoredProfile[], options = {}, drafts: DraftPersister | null = null) {
  const fake = backend(Object.fromEntries(initial.map(item => [item.id, item])), options);
  const session = new ProfileSession(fake.invoke, browserProfile, true, drafts);
  await session.bootstrap();
  return { ...fake, session };
}
const file = (text: string, size = new TextEncoder().encode(text).length) => ({ size, text: async () => text });

describe('import', () => {
  it('rejects a file over 1 MiB without reading it or changing anything', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    let read = false;
    await importFile(session, { size: maximumImportBytes + 1, text: async () => { read = true; return ''; } });
    expect(read).toBe(false);
    expect(session.getState().importError).toMatch(/at most 1 MiB/);
    expect(calls.map(call => call.command)).toEqual(['bootstrap']);
    expect(session.getState().draft.profileId).toBe('a');
  });

  it('shows bad JSON inline and changes nothing', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    await importFile(session, file('{broken'));
    expect(session.getState().importError).toMatch(/not valid JSON/);
    expect(calls.some(call => call.command === 'save_profile')).toBe(false);
  });

  it('shows a backend rejection and does not create a profile', async () => {
    const { session, calls, files } = await open([stored('a', 'Alpha')], { rejectValidation: 'Strict' });
    await importFile(session, file(JSON.stringify({ schema_version: 1, name: 'Strict', bindings: [], extra: 1 })));
    expect(session.getState().importError).toContain('unknown field `extra`');
    expect(calls.some(call => call.command === 'save_profile')).toBe(false);
    expect(files.size).toBe(1);
  });

  it('upgrades a legacy file, validates it, creates a NEW profile and switches to it', async () => {
    const { session, calls, files } = await open([stored('a', 'Alpha')]);
    await importFile(session, file(JSON.stringify({ schema_version: 1, name: 'Desk', bindings: [] })));
    await flush();
    const validated = calls.find(call => call.command === 'validate_profile')!.args!.profile as Profile;
    expect(validated.schema_version).toBe(2);
    const save = calls.find(call => call.command === 'save_profile')!;
    expect(save.args!.request).toMatchObject({ id: null, expectedModifiedAtMs: null });
    const state = session.getState();
    expect(state.draft.profileId).toBe('new-1');
    expect(state.draft.profile.name).toBe('Desk');
    expect(state.notice).toBe('Imported Desk. It is open in the editor, not running on the modules yet. Choose it in the profile picker to run it.');
    expect(state.importError).toBeNull();
    expect(files.get('a')!.profile.name).toBe('Alpha');
    expect(session.unsaved).toBe(false);
  });

  it('suffixes a conflicting name instead of overwriting', async () => {
    const { session, calls } = await open([stored('a', 'Alpha'), stored('b', 'Alpha (imported)')]);
    await importFile(session, file(JSON.stringify({ schema_version: 1, name: 'Alpha', bindings: [] })));
    await flush();
    expect(names(calls, 'save_profile')).toEqual(['Alpha (imported 2)']);
  });

  it('asks before replacing an unsaved draft and imports nothing if the user cancels', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.setAutosave(false);
    session.update(item => ({ ...item, name: 'Edited' }));
    await importFile(session, file(JSON.stringify({ schema_version: 1, name: 'Desk', bindings: [] })));
    expect(session.getState().pendingIntent?.label).toBe('import this profile');
    session.resolveIntent(false);
    await flush();
    expect(calls.some(call => call.command === 'save_profile')).toBe(false);
    expect(session.getState().draft.profile.name).toBe('Edited');
  });

  it('reports a save failure inline', async () => {
    const test = await open([stored('a', 'Alpha')]);
    const failing: Invoke = async (command, args) => { if (command === 'save_profile') throw 'disk full'; return test.invoke(command, args); };
    const session = new ProfileSession(failing, browserProfile, true);
    await session.bootstrap();
    await importFile(session, file(JSON.stringify({ schema_version: 1, name: 'Desk', bindings: [] })));
    await flush();
    expect(session.getState().importError).toBe('Could not import the profile: disk full');
    expect(session.getState().draft.profileId).toBe('a');
  });
});

describe('export', () => {
  it('sends the draft to the backend and shows the path', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.update(item => ({ ...item, name: 'Edited' }));
    await session.exportProfile();
    expect(names(calls, 'export_profile')).toEqual(['Edited']);
    expect(session.getState().exportResult).toEqual({ path: '/s/exports/x-20261005-120000.json', bytes: 1234 });
    expect(session.getState().busy).toBeNull();
    expect(session.getState().draft.profile.name).toBe('Edited');
    const html = renderToStaticMarkup(<ProfileTransfer session={session} state={session.getState()} disabled={false} mac />);
    expect(html).toContain('/s/exports/x-20261005-120000.json');
    expect(html).toContain('Copy path');
    expect(html).toContain('accept=".json,application/json"');
    expect(html).toContain('Create a saved copy of this profile (⌘D)');
  });

  it('names the draft source so an exported edit of an imported profile keeps its provenance', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.update(item => ({ ...item, name: 'Edited' }));
    await session.exportProfile();
    const call = calls.find(item => item.command === 'export_profile')!;
    expect(call.args).toEqual({ request: { profile: expect.objectContaining({ name: 'Edited' }), sourceProfileId: 'a' } });
  });

  it('reports backend errors', async () => {
    const test = await open([stored('a', 'Alpha')]);
    const session = new ProfileSession(async (command, args) => { if (command === 'export_profile') throw 'invalid profile'; return test.invoke(command, args); }, browserProfile, true);
    await session.bootstrap();
    await session.exportProfile();
    expect(session.getState().error).toBe('Could not export profile: invalid profile');
    expect(session.getState().exportResult).toBeNull();
  });

  it('copies the path or explains the fallback', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    expect(await copyText('/p', { writeText })).toBe('Path copied.');
    expect(writeText).toHaveBeenCalledWith('/p');
    expect(await copyText('/p', { writeText: () => Promise.reject(new Error('denied')) })).toBe(copyFallback);
    expect(await copyText('/p', undefined)).toBe(copyFallback);
  });
});

describe('duplicate', () => {
  it('copies a clean profile as "<name> copy" and switches to it', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.requestDuplicate();
    await flush();
    expect(session.getState().duplicatePrompt).toBe(false);
    expect(names(calls, 'save_profile')).toEqual(['Alpha copy']);
    expect(calls.find(call => call.command === 'save_profile')!.args!.request).toMatchObject({ id: null, expectedModifiedAtMs: null, sourceProfileId: 'a' });
    expect(session.getState().draft.profileId).toBe('new-1');
  });

  it('numbers the copy when names are taken', async () => {
    const { session, calls } = await open([stored('a', 'Alpha'), stored('b', 'Alpha copy')]);
    session.requestDuplicate();
    await flush();
    expect(names(calls, 'save_profile')).toEqual(['Alpha copy 2']);
  });

  it('asks which version to copy when the draft is dirty: saved version', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.update(item => ({ ...item, name: 'Alpha edited' }));
    session.requestDuplicate();
    expect(session.getState().duplicatePrompt).toBe(true);
    expect(calls.some(call => call.command === 'save_profile')).toBe(false);
    await session.resolveDuplicate('saved');
    expect(calls.some(call => call.command === 'load_profile')).toBe(true);
    expect(names(calls, 'save_profile')).toEqual(['Alpha copy']);
    expect(session.getState().duplicatePrompt).toBe(false);
  });

  it('asks which version to copy when the draft is dirty: current draft', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.update(item => ({ ...item, name: 'Alpha edited' }));
    session.requestDuplicate();
    await session.resolveDuplicate('draft');
    expect(names(calls, 'save_profile')).toEqual(['Alpha edited copy']);
  });

  it('cancelling the choice changes nothing', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    session.update(item => ({ ...item, name: 'Alpha edited' }));
    session.requestDuplicate();
    await session.resolveDuplicate(null);
    expect(session.getState().duplicatePrompt).toBe(false);
    expect(calls.some(call => call.command === 'save_profile')).toBe(false);
    expect(session.getState().draft.profile.name).toBe('Alpha edited');
  });
});

describe('save shortcut helper', () => {
  it('notes when there is nothing to save and saves otherwise', async () => {
    const { session, calls } = await open([stored('a', 'Alpha')]);
    await session.saveOrNote();
    expect(session.getState().notice).toMatch(/Nothing to save/);
    expect(calls.some(call => call.command === 'save_profile')).toBe(false);
    session.update(item => ({ ...item, name: 'Edited' }));
    await session.saveOrNote();
    expect(names(calls, 'save_profile')).toEqual(['Edited']);
  });
});

describe('crash-recovery drafts in the session', () => {
  const persistence = (storage = new FakeStorage()) => {
    const store = new DraftStore(storage, () => 1);
    const tasks: (() => void)[] = [];
    return { storage, store, tasks, persister: new DraftPersister(store, action => { tasks.push(action); return () => {}; }) };
  };

  it('writes edits through, clears on save, and does not offer anything on a later start', async () => {
    const { store, persister } = persistence();
    const { session } = await open([stored('a', 'Alpha')], {}, persister);
    session.update(item => ({ ...item, name: 'Edited' }));
    session.flushDraft();
    expect(store.read('a')?.profile.name).toBe('Edited');
    expect(store.read('a')?.base_modified_at_ms).toBe(1000);
    await session.save();
    expect(store.read('a')).toBeNull();
  });

  it('offers to restore after a restart; Restore keeps the changes and the old base time', async () => {
    const { store, persister, storage } = persistence();
    store.write('a', profile('Edited'), 900);
    const { session, files } = await open([stored('a', 'Alpha', 1000)], {}, persister);
    const prompt = session.getState().restorePrompt!;
    expect(prompt).toMatchObject({ key: 'a', name: 'Edited', changedOnDisk: true });
    expect(session.unsaved).toBe(false);
    session.resolveRestore(true);
    expect(session.getState().draft.profile.name).toBe('Edited');
    expect(session.getState().draft.modifiedAtMs).toBe(900);
    expect(session.getState().restorePrompt).toBeNull();
    expect(session.unsaved).toBe(true);
    expect(files.get('a')!.profile.name).toBe('Alpha');
    session.flushDraft();
    expect(storage.length).toBe(1);
  });

  it('Discard removes the stored draft and keeps the saved profile', async () => {
    const { store, persister } = persistence();
    store.write('a', profile('Edited'), 1000);
    const { session } = await open([stored('a', 'Alpha', 1000)], {}, persister);
    expect(session.getState().restorePrompt).toMatchObject({ changedOnDisk: false });
    session.resolveRestore(false);
    expect(store.read('a')).toBeNull();
    expect(session.getState().draft.profile.name).toBe('Alpha');
    expect(session.unsaved).toBe(false);
  });

  it('does not offer a draft equal to the saved profile, and purges drafts of deleted profiles', async () => {
    const { store, persister } = persistence();
    store.write('a', profile('Alpha'), 1000);
    store.write('gone', profile('Gone'), 1);
    const { session } = await open([stored('a', 'Alpha')], {}, persister);
    expect(session.getState().restorePrompt).toBeNull();
    expect(store.keys()).toEqual([]);
  });

  it('offers a stored draft when switching profiles', async () => {
    const { store, persister } = persistence();
    const { session } = await open([stored('a', 'Alpha'), stored('b', 'Beta')], {}, persister);
    store.write('b', profile('Beta edited'), 1000);
    await session.load('b');
    expect(session.getState().restorePrompt).toMatchObject({ key: 'b', name: 'Beta edited' });
  });

  it('offers a never-saved draft when the opened profile has none', async () => {
    const { store, persister } = persistence();
    store.write(newDraftKey, profile('Unsaved idea'), null);
    const { session } = await open([stored('a', 'Alpha')], {}, persister);
    expect(session.getState().restorePrompt?.key).toBe(newDraftKey);
    session.resolveRestore(true);
    expect(session.getState().draft.profileId).toBeNull();
    expect(session.getState().draft.profile.name).toBe('Unsaved idea');
  });

  it('explicitly discarding a draft through the guard clears its recovery copy', async () => {
    const { store, persister } = persistence();
    const { session } = await open([stored('a', 'Alpha')], {}, persister);
    session.setAutosave(false);
    session.update(item => ({ ...item, name: 'Edited' }));
    session.flushDraft();
    session.guard('reload the saved profile', () => { void session.load('a'); });
    session.resolveIntent(true);
    await flush();
    expect(store.read('a')).toBeNull();
    expect(session.getState().restorePrompt).toBeNull();
    expect(session.getState().draft.profile.name).toBe('Alpha');
  });

  it('clears the recovery copy of a profile that was deleted', async () => {
    const { store, persister } = persistence();
    const { session } = await open([stored('a', 'Alpha')], {}, persister);
    session.update(item => ({ ...item, name: 'Edited' }));
    session.flushDraft();
    await session.deleteProfile({ id: 'a', name: 'Alpha', binding_count: 0, modified_at_ms: 1000, dry_run_only: false });
    expect(store.read('a')).toBeNull();
    expect(session.getState().draft.profileId).toBeNull();
  });

  it('a recovered draft of a deleted profile keeps its source', async () => {
    const { store, persister } = persistence();
    const id = '00000000-0000-4000-8000-0000000000aa';
    const first = await open([stored(id, 'Alpha')], {}, persister);
    await first.session.deleteProfile({ id, name: 'Alpha', binding_count: 0, modified_at_ms: 1000, dry_run_only: false });
    first.session.update(item => ({ ...item, name: 'Edited after delete' }));
    first.session.flushDraft();
    expect(store.read(newDraftKey)).not.toBeNull();
    const second = await open([], {}, new DraftPersister(store, action => { action(); return () => {}; }));
    expect(second.session.getState().restorePrompt?.key).toBe(newDraftKey);
    second.session.resolveRestore(true);
    expect(second.session.getState().draft.profile.name).toBe('Edited after delete');
    expect(second.session.getState().draft.sourceId).toBe(id);
  });

  it('works with no storage at all', async () => {
    const persister = new DraftPersister(new DraftStore(null));
    const { session } = await open([stored('a', 'Alpha')], {}, persister);
    session.update(item => ({ ...item, name: 'Edited' }));
    expect(() => session.flushDraft()).not.toThrow();
    expect(session.getState().restorePrompt).toBeNull();
    await session.save();
    expect(session.unsaved).toBe(false);
  });
});
