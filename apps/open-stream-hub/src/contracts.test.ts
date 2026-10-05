import { describe, expect, it } from 'vitest';
import contracts from '../fixtures/ipc-contracts.json';
import legacyExpected from '../fixtures/legacy-upgrade-expected.json';
import legacyInput from '../fixtures/legacy-upgrade-input.json';
import type { AppBootstrap, KeypadPreviewResult, Profile, ProfileListing, ProfileSummary, ReplayEvent, ReplayResult, StoredProfile, UnreadableProfile } from './contracts';
import { upgradeKeypadProfile } from './keypad-model';

// Each list must name every key of its TypeScript type (checked at compile
// time) and match the keys Rust serializes (checked against the shared
// fixture, which a Rust test regenerates from the real types).
type Exact<T, K extends readonly (keyof T)[]> = [Exclude<keyof T, K[number]>] extends [never] ? K : never;
const keys = <T,>() => <const K extends readonly (keyof T)[]>(list: Exact<T, K>) => list as readonly string[];

const shapes = {
  storedProfile: keys<StoredProfile>()(['id', 'profile', 'modified_at_ms']),
  summary: keys<ProfileSummary>()(['id', 'name', 'binding_count', 'modified_at_ms']),
  unreadable: keys<UnreadableProfile>()(['id', 'error']),
  listing: keys<ProfileListing>()(['profiles', 'unreadable']),
  bootstrap: keys<AppBootstrap>()(['profile', 'profiles', 'unreadable', 'storage_path', 'desktop']),
  keypadPreview: keys<KeypadPreviewResult>()(['path', 'depth', 'events', 'planned_action_count']),
  replayResult: keys<ReplayResult>()(['source', 'synthetic', 'status', 'report_count', 'event_count', 'planned_action_count', 'events', 'diagnostics']),
  event: keys<ReplayEvent>()(['timestamp_us', 'module', 'control', 'gesture', 'value', 'serial_number', 'action', 'page_id', 'navigation']),
};
const optionalEventKeys = ['value', 'page_id', 'navigation'];

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
    expect(contracts.stored_profile.profile.schema_version).toBe(2);
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

describe('legacy keypad upgrade parity with Rust into_v2', () => {
  it('produces the profile Rust produces for the shared fixture', () => {
    const input = structuredClone(legacyInput) as Profile;
    expect(upgradeKeypadProfile(input)).toEqual(legacyExpected);
    expect(input).toEqual(legacyInput);
  });

  it('leaves an already upgraded profile unchanged', () => {
    expect(upgradeKeypadProfile(legacyExpected as Profile)).toEqual(legacyExpected);
  });
});
