import { describe, expect, it } from 'vitest';
import { appDisplayName, appNamesVersion, rememberAppName, subscribeAppNames } from './app-names';

describe('app names', () => {
  it('falls back to the bundle ID, matches case-insensitively and tells listeners only about new names', () => {
    expect(appDisplayName('test.names.app')).toBe('test.names.app');
    let calls = 0;
    const stop = subscribeAppNames(() => { calls += 1; });
    const before = appNamesVersion();
    rememberAppName('Test.Names.App', ' Names ');
    expect(appDisplayName('test.names.app')).toBe('Names');
    expect(appNamesVersion()).toBe(before + 1);
    rememberAppName('test.names.app', 'Names');
    rememberAppName('test.names.app', null);
    expect(calls).toBe(1);
    stop();
    rememberAppName('test.names.app', 'Renamed');
    expect(calls).toBe(1);
  });
});
