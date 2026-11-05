import { describe, expect, it } from 'vitest';
import type { BindingAction } from './contracts';
import { rememberAppName } from './app-names';
import { followedKeyLabel } from './useDeviceEditor';

const open = (bundle_id: string) => ({ type: 'launch_app', bundle_id }) as BindingAction;

describe('followedKeyLabel', () => {
  it('moves an automatic label (the name, or the bundle ID before the name was known) to the new app', () => {
    rememberAppName('test.finder', 'Finder');
    rememberAppName('test.mail', 'Mail');
    expect(followedKeyLabel('Finder', open('test.finder'), open('test.mail'))).toBe('Mail');
    expect(followedKeyLabel('test.finder', open('test.finder'), open('test.mail'))).toBe('Mail');
  });
  it('leaves a label the user wrote, and does nothing without a change of app', () => {
    rememberAppName('test.finder', 'Finder');
    expect(followedKeyLabel('My files', open('test.finder'), open('test.mail'))).toBeNull();
    expect(followedKeyLabel('Finder', open('test.finder'), open('test.finder'))).toBeNull();
    expect(followedKeyLabel('Finder', undefined, open('test.mail'))).toBeNull();
    expect(followedKeyLabel('Finder', open('test.finder'), { type: 'toggle_mute' } as BindingAction)).toBeNull();
  });
});
