import { describe, expect, it } from 'vitest';
import contracts from '../fixtures/ipc-contracts.json';
import legacyExpected from '../fixtures/legacy-upgrade-expected.json';
import legacyInput from '../fixtures/legacy-upgrade-input.json';
import exportRequest from '../fixtures/profile-export-request.json';
import exportResult from '../fixtures/profile-export-result.json';
import mhzImport from '../fixtures/mhz-import.json';
import type { AppBootstrap, ImportCategoryCounts, ImportCounts, ImportedProfile, ImportReport, ImportReportRow, KeypadPreviewResult, MhzImportResult, Profile, ProfileListing, ProfileSummary, ReplayEvent, ReplayResult, StoredProfile, UnreadableProfile } from './contracts';
import { upgradeKeypadProfile } from './keypad-model';

// Each list must name every key of its TypeScript type (checked at compile
// time) and match the keys Rust serializes (checked against the shared
// fixture, which a Rust test regenerates from the real types).
type Exact<T, K extends readonly (keyof T)[]> = [Exclude<keyof T, K[number]>] extends [never] ? K : never;
const keys = <T,>() => <const K extends readonly (keyof T)[]>(list: Exact<T, K>) => list as readonly string[];

const shapes = {
  storedProfile: keys<StoredProfile>()(['id', 'profile', 'modified_at_ms', 'content_hash']),
  summary: keys<ProfileSummary>()(['id', 'name', 'binding_count', 'modified_at_ms', 'dry_run_only']),
  unreadable: keys<UnreadableProfile>()(['id', 'error']),
  listing: keys<ProfileListing>()(['profiles', 'unreadable', 'import_flags_error']),
  bootstrap: keys<AppBootstrap>()(['profile', 'profiles', 'unreadable', 'import_flags_error', 'storage_path', 'desktop']),
  keypadPreview: keys<KeypadPreviewResult>()(['path', 'depth', 'events', 'planned_action_count']),
  replayResult: keys<ReplayResult>()(['source', 'synthetic', 'status', 'report_count', 'event_count', 'planned_action_count', 'events', 'diagnostics']),
  event: keys<ReplayEvent>()(['timestamp_us', 'module', 'control', 'gesture', 'value', 'serial_number', 'action', 'page_id', 'navigation', 'dial_function', 'dial_function_switch']),
};
const optionalEventKeys = ['value', 'page_id', 'navigation', 'dial_function', 'dial_function_switch'];

const sorted = (value: object) => Object.keys(value).sort();
const expectKeys = (value: object, list: readonly string[]) => expect(sorted(value)).toEqual([...list].sort());
function expectEvent(event: object) {
  const present = sorted(event);
  expect(present.every(key => shapes.event.includes(key))).toBe(true);
  expect(shapes.event.filter(key => !optionalEventKeys.includes(key)).every(key => present.includes(key))).toBe(true);
}

describe('Rust IPC contract fixture', () => {
  it('StoredProfile has exactly the fields the frontend reads', () => {
    expectKeys(contracts.stored_profile, shapes.storedProfile);
    expect(typeof contracts.stored_profile.modified_at_ms).toBe('number');
    expect(contracts.stored_profile.profile.schema_version).toBe(4);
  });

  it('bootstrap and listing include readable and unreadable profiles', () => {
    expectKeys(contracts.app_bootstrap, shapes.bootstrap);
    expectKeys(contracts.app_bootstrap.profile!, shapes.storedProfile);
    expectKeys(contracts.profile_listing, shapes.listing);
    for (const item of contracts.profile_listing.profiles) expectKeys(item, shapes.summary);
    expect(contracts.profile_listing.unreadable.length).toBeGreaterThan(0);
    for (const item of contracts.profile_listing.unreadable) expectKeys(item, shapes.unreadable);
  });

  it('KeypadPreview carries both click events and navigation', () => {
    expectKeys(contracts.keypad_preview, shapes.keypadPreview);
    expect(contracts.keypad_preview.events).toHaveLength(2);
    contracts.keypad_preview.events.forEach(expectEvent);
    expect(contracts.keypad_preview.events.some(event => 'navigation' in event)).toBe(true);
  });

  it('ReplayResult matches the replay view fields', () => {
    expectKeys(contracts.replay_result, shapes.replayResult);
    expect(contracts.replay_result.synthetic).toBe(true);
    expect(contracts.replay_result.status).toBe('completed');
    contracts.replay_result.events.forEach(expectEvent);
    for (const diagnostic of contracts.replay_result.diagnostics as object[]) expectKeys(diagnostic, ['timestamp_us', 'message']);
  });

  it('the frontend can consume the fixture values as its declared types', () => {
    const replay = contracts.replay_result as unknown as ReplayResult;
    expect(replay.event_count).toBe(replay.events.length);
    const preview = contracts.keypad_preview as unknown as KeypadPreviewResult;
    expect(preview.path[0]).toBe('root');
  });
});

describe('legacy keypad upgrade parity with Rust into_current', () => {
  it('produces the profile Rust produces for the shared fixture', () => {
    const input = structuredClone(legacyInput) as Profile;
    expect(upgradeKeypadProfile(input)).toEqual(legacyExpected);
    expect(input).toEqual(legacyInput);
  });

  it('leaves an already upgraded profile unchanged', () => {
    expect(upgradeKeypadProfile(legacyExpected as Profile)).toEqual(legacyExpected);
  });
});

describe('profile export fixtures', () => {
  it('the request carries one upgraded profile and the result a path and byte count', () => {
    expect(Object.keys(exportRequest)).toEqual(['request']);
    expect(Object.keys(exportRequest.request).sort()).toEqual(['profile', 'sourceProfileId']);
    expect(exportRequest.request.profile.schema_version).toBe(4);
    expect(Object.keys(exportResult.export_profile).sort()).toEqual(['bytes', 'path']);
    expect(exportResult.export_profile.path).toMatch(/\/exports\/[a-z0-9-]+-\d{8}-\d{6}\.json$/);
    expect(typeof exportResult.export_profile.bytes).toBe('number');
  });
});

describe('official .mhz import fixture', () => {
  const report = keys<ImportReport>()(['profile_name', 'source_name', 'totals', 'categories', 'actions', 'high_risk', 'warnings', 'rows']);
  const counts = keys<ImportCounts>()(['imported', 'imported_lossy', 'unbound_unsupported', 'unbound_dangerous']);
  const category = keys<ImportCategoryCounts>()(['category', 'imported', 'imported_lossy', 'unbound_unsupported', 'unbound_dangerous']);
  const row = keys<ImportReportRow>()(['section', 'path', 'official_action', 'outcome', 'reason', 'detail', 'high_risk']);

  it('import_mhz_profile returns new dry-run-only profiles with a redacted report', () => {
    expectKeys(mhzImport.import_mhz_profile, keys<MhzImportResult>()(['profiles', 'source_sha256']));
    const imported = mhzImport.import_mhz_profile.profiles[0] as unknown as ImportedProfile;
    expectKeys(imported, keys<ImportedProfile>()(['stored', 'report']));
    expectKeys(imported.stored, shapes.storedProfile);
    expectKeys(imported.report, report);
    expectKeys(imported.report.totals, counts);
    for (const item of [...imported.report.categories, ...imported.report.actions]) expectKeys(item, category);
    for (const item of imported.report.rows) {
      const present = sorted(item);
      expect(present.every(key => row.includes(key))).toBe(true);
      expect(row.filter(key => key !== 'detail').every(key => present.includes(key))).toBe(true);
    }
    expect(imported.stored.profile.name).toBe('Desk (imported)');
    const text = JSON.stringify(mhzImport.import_mhz_profile);
    expect(text).not.toMatch(/FAKE|Tools/);
  });

  it('allow_imported_profile_execution clears the flag and returns the listing', () => {
    expectKeys(mhzImport.allow_imported_profile_execution, shapes.listing);
    for (const item of mhzImport.allow_imported_profile_execution.profiles) expectKeys(item, shapes.summary);
    expect(mhzImport.allow_imported_profile_execution.profiles.every(item => item.dry_run_only === false)).toBe(true);
    // The exact argument shapes the Rust test parsed with its strict (deny_unknown_fields) request structs.
    expect(Object.keys(mhzImport.allow_imported_profile_execution_args.request).sort()).toEqual(['contentHash', 'id']);
    expect(Object.keys(mhzImport.save_profile_args.request).sort()).toEqual(['expectedModifiedAtMs', 'id', 'profile', 'sourceProfileId']);
    expect(mhzImport.save_profile_args.request.sourceProfileId).toBe(mhzImport.allow_imported_profile_execution_args.request.id);
  });

  it('a damaged import list marks every profile and names the recovery', () => {
    expectKeys(mhzImport.damaged_listing, shapes.listing);
    expect(mhzImport.damaged_listing.import_flags_error).toBeTruthy();
    expect(mhzImport.damaged_listing.profiles.every(item => item.dry_run_only)).toBe(true);
  });
});
