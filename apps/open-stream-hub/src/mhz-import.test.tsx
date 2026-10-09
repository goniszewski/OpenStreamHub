import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import fixture from '../fixtures/mhz-import.json';
import type { MhzImportResult, Profile, ProfileSummary, StoredProfile } from './contracts';
import { upgradeKeypadProfile } from './keypad-model';
import { AllowExecutionDialog, allowCopy, countsLine, MhzImportReport } from './MhzImport';
import { isMhzFileName, maximumMhzBytes } from './profile-library';
import { draftDryRunOnly, draftRunBlock, ProfileSession } from './profile-session';
import type { Invoke } from './profile-session';
import { ProfileManagerSheet } from './ProfileManager';
import { importDropped, importMhzFile, ProfileTransfer } from './ProfileTransfer';
import { dryRunOnlyRunNote, RunHeaderControl, ServicePanel, useRunSession } from './ServicePanel';
import { ServiceRunner } from './service-runner';
import { browserProfile } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));
const result = fixture.import_mhz_profile as unknown as MhzImportResult;
const imported = result.profiles[0].stored;
const plain: StoredProfile = { id: 'a', profile: upgradeKeypadProfile({ schema_version: 1, name: 'Alpha', bindings: [] }), modified_at_ms: 1000 };

// The Rust commands the import touches, in memory.
function backend(options: { rejectImport?: string; rejectAllow?: string; damaged?: boolean } = {}) {
  const files = new Map<string, StoredProfile>([[plain.id, plain]]);
  const flagged = new Set<string>();
  let damaged = options.damaged ?? false;
  const listing = () => ({ profiles: summaries().map(item => ({ ...item, dry_run_only: damaged || item.dry_run_only })), unreadable: [], import_flags_error: damaged ? 'the imported profile list is unreadable' : null });
  const calls: { command: string; args?: unknown }[] = [];
  const summaries = (): ProfileSummary[] => [...files.values()].map(item => ({ id: item.id, name: item.profile.name, binding_count: 0, modified_at_ms: item.modified_at_ms, dry_run_only: flagged.has(item.id) }));
  const invoke: Invoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ command, args });
    switch (command) {
      case 'bootstrap': return { profile: plain, ...listing(), storage_path: '/s', desktop: true } as T;
      case 'list_profiles': return listing() as T;
      case 'mark_all_profiles_for_review':
        damaged = false;
        for (const id of files.keys()) flagged.add(id);
        return listing() as T;
      case 'load_profile': return files.get(args!.id as string) as T;
      case 'import_mhz_profile':
        if (options.rejectImport) throw options.rejectImport;
        files.set(imported.id, imported);
        flagged.add(imported.id);
        return result as T;
      case 'allow_imported_profile_execution':
        if (options.rejectAllow) throw options.rejectAllow;
        flagged.delete((args!.request as { id: string }).id);
        return listing() as T;
      case 'save_profile': {
        const request = args!.request as { id: string | null; profile: Profile; sourceProfileId: string | null };
        const saved = { id: request.id ?? 'copy-1', profile: request.profile, modified_at_ms: 3000 };
        files.set(saved.id, saved);
        if (request.sourceProfileId && flagged.has(request.sourceProfileId)) flagged.add(saved.id);
        return saved as T;
      }
      case 'delete_profile': files.delete(args!.id as string); return null as T;
      default: throw new Error(`unexpected ${command}`);
    }
  };
  return { calls, invoke, flagged };
}

async function open(options = {}) {
  const fake = backend(options);
  const session = new ProfileSession(fake.invoke, browserProfile, true);
  await session.bootstrap();
  return { ...fake, session };
}

const mhzFile = (bytes: number[], size = bytes.length, name = 'Desk.mhz') => ({ name, size, text: async () => '', arrayBuffer: async () => new Uint8Array(bytes).buffer });

describe('official .mhz import', () => {
  it('recognises official file names and refuses oversize files without reading them', async () => {
    for (const name of ['a.mhz', 'B.MHZ', 'masterHubConfig_profiles.zip ']) expect(isMhzFileName(name)).toBe(true);
    for (const name of ['a.json', 'a.mhz.txt', 'mhz']) expect(isMhzFileName(name)).toBe(false);
    const { session, calls } = await open();
    let read = false;
    await importMhzFile(session, { size: maximumMhzBytes + 1, arrayBuffer: async () => { read = true; return new ArrayBuffer(0); } });
    expect(read).toBe(false);
    expect(session.getState().importError).toMatch(/at most 32 MiB/);
    expect(calls.map(call => call.command)).toEqual(['bootstrap']);
  });

  it('sends the raw bytes, opens the new dry-run-only profile and keeps the report', async () => {
    const { session, calls } = await open();
    await importMhzFile(session, mhzFile([0x50, 0x4b, 3, 4]));
    await flush();
    const call = calls.find(item => item.command === 'import_mhz_profile')!;
    expect(call.args).toBeInstanceOf(Uint8Array);
    expect([...(call.args as Uint8Array)]).toEqual([0x50, 0x4b, 3, 4]);
    const state = session.getState();
    expect(state.draft.profileId).toBe(imported.id);
    expect(state.draft.profile.name).toBe('Desk (imported)');
    expect(state.mhzImport).toEqual(result);
    expect(state.notice).toMatch(/run only as a dry run/);
    expect(state.profiles.find(item => item.id === imported.id)?.dry_run_only).toBe(true);
    expect(calls.some(item => item.command === 'save_profile')).toBe(false);
    session.dismissMhzImport();
    expect(session.getState().mhzReportOpen).toBe(false);
  });

  it('routes dropped files by name and shows a backend refusal inline', async () => {
    const { session, calls } = await open({ rejectImport: 'the archive contains an unsafe entry name' });
    await importDropped(session, mhzFile([1, 2, 3], 3, 'evil.zip'));
    await flush();
    expect(calls.map(call => call.command)).toContain('import_mhz_profile');
    expect(session.getState().importError).toBe('Could not import the official profile: the archive contains an unsafe entry name');
    expect(calls.filter(call => call.command === 'list_profiles').length).toBeGreaterThan(0);
    expect(session.getState().draft.profileId).toBe('a');
    const json = { name: 'p.json', size: 2, text: async () => '{', arrayBuffer: async () => new ArrayBuffer(0) };
    await importDropped(session, json);
    expect(session.getState().importError).toMatch(/not valid JSON/);
  });

  it('asks before replacing an unsaved draft', async () => {
    const { session, calls } = await open();
    session.setAutosave(false);
    session.update(item => ({ ...item, name: 'Edited' }));
    await importMhzFile(session, mhzFile([1]));
    expect(session.getState().pendingIntent?.label).toBe('import this profile');
    session.resolveIntent(false);
    await flush();
    expect(calls.some(call => call.command === 'import_mhz_profile')).toBe(false);
  });

  it('a duplicate of a flagged profile names its source so the flag is inherited', async () => {
    const { session, calls, flagged } = await open();
    await importMhzFile(session, mhzFile([1]));
    await flush();
    session.requestDuplicate();
    await flush();
    const save = calls.find(call => call.command === 'save_profile')!;
    expect(save.args).toEqual({ request: expect.objectContaining({ id: null, sourceProfileId: imported.id }) });
    expect(flagged.has('copy-1')).toBe(true);
  });

  it('allows actions only through the explicit call, for the version the user reviewed', async () => {
    const { session, calls } = await open();
    await importMhzFile(session, mhzFile([1]));
    await flush();
    expect(session.getState().draft.contentHash).toBe(imported.content_hash);
    expect(await session.allowExecution(imported.id, 'Desk (imported)', imported.content_hash!)).toBe(true);
    expect(calls.find(call => call.command === 'allow_imported_profile_execution')!.args).toEqual({ request: { id: imported.id, contentHash: imported.content_hash } });
    expect(session.getState().profiles.every(item => !item.dry_run_only)).toBe(true);
    expect(session.getState().notice).toMatch(/may now run with actions/);
  });

  it('keeps the source of a deleted open profile for later saves', async () => {
    const { session, calls } = await open();
    await importMhzFile(session, mhzFile([1]));
    await flush();
    await session.deleteProfile(session.getState().profiles.find(item => item.id === imported.id)!);
    expect(session.getState().draft.profileId).toBeNull();
    expect(session.getState().draft.sourceId).toBe(imported.id);
    session.update(item => ({ ...item, name: 'Edited after delete' }));
    await session.save();
    const save = calls.filter(call => call.command === 'save_profile').at(-1)!;
    expect(save.args).toEqual({ request: expect.objectContaining({ id: null, sourceProfileId: imported.id }) });
  });

  it('a damaged import list is shown and recovered only by marking everything for review', async () => {
    const { session, calls } = await open({ damaged: true });
    expect(session.getState().importFlagsError).toMatch(/unreadable/);
    expect(session.getState().profiles.every(item => item.dry_run_only)).toBe(true);
    expect(await session.markAllForReview()).toBe(true);
    expect(calls.map(call => call.command)).toContain('mark_all_profiles_for_review');
    expect(session.getState().importFlagsError).toBeNull();
    expect(session.getState().profiles.every(item => item.dry_run_only)).toBe(true);
  });

  it('keeps the last report and reopens it', async () => {
    const { session } = await open();
    await importMhzFile(session, mhzFile([1]));
    await flush();
    expect(session.getState().mhzReportOpen).toBe(true);
    session.dismissMhzImport();
    expect(session.getState().mhzReportOpen).toBe(false);
    expect(session.getState().mhzImport).toEqual(result);
    session.showMhzImport();
    expect(session.getState().mhzReportOpen).toBe(true);
  });

  it('reports a refused allow and keeps the flag', async () => {
    const { session } = await open({ rejectAllow: 'this profile changed on disk' });
    await importMhzFile(session, mhzFile([1]));
    await flush();
    expect(await session.allowExecution(imported.id, 'Desk (imported)', 'stale')).toBe(false);
    expect(session.getState().error).toBe('Could not allow actions: this profile changed on disk');
    expect(session.getState().profiles.find(item => item.id === imported.id)?.dry_run_only).toBe(true);
  });
});

describe('import UI', () => {
  it('offers the official import next to the JSON import', async () => {
    const { session } = await open();
    const html = renderToStaticMarkup(<ProfileTransfer session={session} state={session.getState()} disabled={false} mac />);
    expect(html).toContain('Import official .mhz');
    expect(html).toContain('accept=".mhz,.zip"');
  });

  it('the report lists counts, warnings, redacted rows and the dry-run note', () => {
    const html = renderToStaticMarkup(<MhzImportReport result={result} onClose={() => {}} />);
    expect(html).toContain('Imported from MasterHUB');
    expect(html).toContain('Desk (imported)');
    expect(html).toContain(countsLine(result.profiles[0].report.totals));
    expect(html).toContain('Left unbound (dangerous)');
    expect(html).toContain('tool.exe');
    expect(html).toContain('https://example.com');
    expect(html).toContain('Opens web pages: https://example.com.');
    expect(html).toContain('run only as a dry run');
    expect(html).not.toMatch(/Tools|FAKE/);
  });

  it('the allow confirmation names the profile, lists its capabilities and needs an acknowledgement', () => {
    const copy = allowCopy('Desk (imported)', imported.profile);
    expect(copy.title).toBe('Allow "Desk (imported)" to execute actions?');
    expect(copy.lines).toContain('Opens web pages: https://example.com.');
    const html = renderToStaticMarkup(<AllowExecutionDialog target={{ name: 'Desk (imported)', profile: imported.profile }} onCancel={() => {}} onConfirm={() => {}} />);
    expect(html).toContain('Allow &quot;Desk (imported)&quot; to execute actions?');
    expect(html).toContain('I reviewed every binding of this imported profile and trust its source');
    expect(html).toMatch(/<button type="button" class="button danger" disabled="">Allow actions<\/button>/);
    const high = upgradeKeypadProfile({ schema_version: 1, name: 'H', bindings: [{ module: 'm003k', control: 0, gesture: 'clockwise', action: { type: 'text', text: 'rm -rf ~', press_enter: true } }] });
    expect(allowCopy('H', high).warning).toMatch(/^High risk/);
  });

  it('the profile manager shows the badge and the explicit allow action for a flagged profile', async () => {
    const { session } = await open();
    await importMhzFile(session, mhzFile([1]));
    await flush();
    session.dismissMhzImport();
    const render = () => renderToStaticMarkup(<ProfileManagerSheet session={session} state={session.getState()} desktop disabled={false} confirm={() => {}} onAllow={() => {}} onClose={() => {}} />);
    let html = render();
    expect(html).toContain('Imported · dry run only');
    expect(html).toContain('Allow this imported profile to execute actions…');
    session.update(item => ({ ...item, name: 'Edited' }));
    html = render();
    expect(html).toContain('Save or discard your changes first');
    expect(html).toMatch(/disabled="">Allow this imported profile/);
    await session.load('a');
    expect(render()).not.toContain('Allow this imported profile');
  });

  it('the profile manager offers the recovery for a damaged list and reopens the last report', async () => {
    const damaged = await open({ damaged: true });
    const page = (session: ProfileSession) => renderToStaticMarkup(<ProfileManagerSheet session={session} state={session.getState()} desktop disabled={false} confirm={() => {}} onAllow={() => {}} onClose={() => {}} />);
    expect(page(damaged.session)).toContain('Mark all profiles as needing review');
    expect(page(damaged.session)).toContain('Every profile runs only as a dry run');
    const { session } = await open();
    await importMhzFile(session, mhzFile([1]));
    await flush();
    session.dismissMhzImport();
    expect(page(session)).toContain('Show import report');
    expect(page(session)).not.toContain('Mark all profiles as needing review');
  });

  it('a Start names the draft\'s source profile even when it is edited', async () => {
    const calls: { command: string; args?: Record<string, unknown> }[] = [];
    const runner = new ServiceRunner(async <T,>(command: string, args?: Record<string, unknown>) => { calls.push({ command, args }); return 'service:aa' as T; }, async () => () => {}, true);
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'P', bindings: [] });
    await runner.start({ profile, pathHex: 'aa', execute: false, softwareMode: false, draftUnsaved: true, profileId: 'src', sourceProfileId: 'src' });
    const request = (calls[0].args as { request: Record<string, unknown> }).request;
    expect(request.source_profile_id).toBe('src');
    expect(request.profile_id).toBeUndefined();
  });

  it('the Run panel offers only a dry run for a flagged profile', () => {
    const runner = new ServiceRunner(async () => null as never, async () => () => {}, true);
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'P', bindings: [] });
    const html = (dryRunOnly: boolean) => {
      const View = () => {
        const run = useRunSession({ runner, profile, desktop: true, unsaved: false, dryRunOnly, platform: { platform: 'MacIntel' }, now: 1 });
        return <><RunHeaderControl run={run} /><ServicePanel run={run} /></>;
      };
      return renderToStaticMarkup(<View />);
    };
    expect(html(true)).toContain(dryRunOnlyRunNote);
    expect(html(true)).toMatch(/disabled="">Execute actions<\/button>/);
    expect(html(false)).not.toContain(dryRunOnlyRunNote);
    expect(html(false)).toMatch(/aria-pressed="false">Execute actions<\/button>/);
  });
});

describe('dry-run-only draft in the Run panel', () => {
  const summary = (id: string, flagged: boolean): ProfileSummary => ({ id, name: id, binding_count: 0, modified_at_ms: 1, dry_run_only: flagged });
  const draft = (profileId: string | null, sourceId: string | null) => ({ profile: plain.profile, profileId, savedSnapshot: '', modifiedAtMs: null, sourceId });
  it('treats a draft whose source profile is gone (deleted) as dry run only', () => {
    expect(draftDryRunOnly({ importFlagsError: null, unreadable: [], profiles: [summary('a', false)], draft: draft(null, 'deleted') })).toBe(true);
    expect(draftDryRunOnly({ importFlagsError: null, unreadable: [], profiles: [summary('a', false)], draft: draft('a', 'a') })).toBe(false);
    expect(draftDryRunOnly({ importFlagsError: null, unreadable: [], profiles: [summary('a', true)], draft: draft(null, 'a') })).toBe(true);
    expect(draftDryRunOnly({ importFlagsError: null, unreadable: [], profiles: [], draft: draft(null, null) })).toBe(false);
    expect(draftDryRunOnly({ importFlagsError: 'damaged', unreadable: [], profiles: [], draft: draft(null, null) })).toBe(true);
  });
});

describe('why Execute is unavailable', () => {
  const summary = (id: string, flagged: boolean): ProfileSummary => ({ id, name: id, binding_count: 0, modified_at_ms: 1, dry_run_only: flagged });
  const draft = (profileId: string | null, sourceId: string | null) => ({ profile: plain.profile, profileId, savedSnapshot: '', modifiedAtMs: null, sourceId });
  it('names a deleted source, a damaged list and an imported profile apart', () => {
    const base = { importFlagsError: null, unreadable: [] as { id: string; error: string }[] };
    expect(draftRunBlock({ ...base, profiles: [summary('a', false)], draft: draft(null, 'gone') })).toBe('deleted');
    expect(draftRunBlock({ ...base, profiles: [summary('a', true)], draft: draft('a', 'a') })).toBe('imported');
    expect(draftRunBlock({ ...base, importFlagsError: 'x', profiles: [], draft: draft(null, null) })).toBe('unreadable');
    expect(draftRunBlock({ ...base, profiles: [], draft: draft(null, null) })).toBeNull();
    // An unreadable saved profile still exists: its draft is not "deleted".
    expect(draftRunBlock({ ...base, unreadable: [{ id: 'u', error: 'bad' }], profiles: [], draft: draft(null, 'u') })).toBeNull();
  });
});
