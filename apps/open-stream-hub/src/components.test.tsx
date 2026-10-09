import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { CrashScreen, ErrorBoundary, IntegerField, LabelError, ModalDialog } from './controls';
import { KeypadScreen } from './KeypadScreen';
import { createKeypadFolder, keypadPagesAtPath, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import { ProfilesPage } from './ProfilesPage';
import { browserProfile } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

describe('keypad breadcrumbs', () => {
  it('shows the root page once, followed by each folder', () => {
    let profile = upgradeKeypadProfile({ schema_version: 1, name: 'Desk profile', bindings: [] });
    profile = createKeypadFolder(profile, ['root'], 1, 'tools', 'Tools');
    const path = ['root', 'tools'];
    const markup = renderToStaticMarkup(<KeypadScreen pages={keypadPagesAtPath(profile, path)} allPages={profile.keypad!.pages} path={path} selected={1} onSelect={() => {}} onNavigate={() => {}} />);
    const crumbs = markup.slice(markup.indexOf('keypad-breadcrumbs'), markup.indexOf('</nav>'));
    expect(crumbs.match(/<button/g)).toHaveLength(2);
    expect(crumbs.match(/>Root</g)).toHaveLength(1);
    expect(crumbs).not.toContain('Desk profile');
    expect(crumbs.match(/lucide-chevron-right/g)).toHaveLength(1);
  });
});

describe('form controls', () => {
  it('renders a valid integer without an error', () => {
    const markup = renderToStaticMarkup(<IntegerField id="volume-delta" value={5} minimum={-100} maximum={100} nonZero onCommit={() => {}} />);
    expect(markup).toContain('value="5"');
    expect(markup).toContain('aria-invalid="false"');
    expect(markup).not.toContain('field-error');
  });

  it('shows inline label errors only for invalid values', () => {
    expect(renderToStaticMarkup(<LabelError id="name-error" value="" field="Profile name" />)).toContain('Profile name cannot be empty.');
    expect(renderToStaticMarkup(<LabelError id="name-error" value="Work" field="Profile name" />)).toBe('');
  });

  it('renders dialog content without opening it during render', () => {
    const markup = renderToStaticMarkup(<ModalDialog open labelledBy="title" onCancel={() => {}}><h2 id="title">Delete?</h2></ModalDialog>);
    expect(markup).toContain('<dialog');
    expect(markup).not.toContain(' open');
  });
});

describe('error boundary', () => {
  it('captures render errors and shows a recovery screen', () => {
    expect(ErrorBoundary.getDerivedStateFromError(new Error('boom')).error.message).toBe('boom');
    expect(ErrorBoundary.getDerivedStateFromError('text').error.message).toBe('text');
    const markup = renderToStaticMarkup(<CrashScreen error={new Error('boom')} />);
    expect(markup).toContain('Something went wrong');
    expect(markup).toContain('boom');
    expect(markup).toContain('Reload window');
  });
});

describe('profiles page', () => {
  it('lists unreadable profiles with the storage folder and offers delete for saved profiles', () => {
    const session = new ProfileSession(() => new Promise(() => {}), browserProfile, false);
    const base = session.getState();
    const state = {
      ...base, storagePath: '/store/profiles',
      profiles: [{ id: 'a', name: 'Alpha', binding_count: 2, modified_at_ms: 1, dry_run_only: false }],
      unreadable: [{ id: 'bad', error: 'invalid profile JSON' }],
    };
    const markup = renderToStaticMarkup(<ProfilesPage session={session} state={state} editor={{} as DeviceEditor} desktop={false} disabled replayKind="knobs" setReplayKind={() => {}} setPage={() => {}} confirm={() => {}} />);
    expect(markup).toContain('Unreadable profiles');
    expect(markup).toContain('bad.json');
    expect(markup).toContain('invalid profile JSON');
    expect(markup).toContain('/store/profiles');
    expect(markup).toContain('aria-label="Delete profile Alpha"');
  });

  it('keeps the sample-input test as a collapsed section with its summary visible', () => {
    const session = new ProfileSession(() => new Promise(() => {}), browserProfile, false);
    const markup = renderToStaticMarkup(<ProfilesPage session={session} state={session.getState()} editor={{} as DeviceEditor} desktop={false} disabled replayKind="knobs" setReplayKind={() => {}} setPage={() => {}} confirm={() => {}} />);
    expect(markup).toContain('<details class="panel replay-panel">');
    expect(markup).toContain('<summary class="replay-summary"><span>Test with sample input</span><span class="small-label">No hardware required</span></summary>');
    expect(markup).toContain('>Run test</button>');
  });

  it('offers Remove for keypad bindings', () => {
    const session = new ProfileSession(() => new Promise(() => {}), { schema_version: 1, name: 'Keys', bindings: [{ module: 'm4315', control: 2, gesture: 'pressed', action: { type: 'toggle_mute' } }] }, false);
    const markup = renderToStaticMarkup(<ProfilesPage session={session} state={session.getState()} editor={{} as DeviceEditor} desktop={false} disabled replayKind="knobs" setReplayKind={() => {}} setPage={() => {}} confirm={() => {}} />);
    expect(markup).toContain('aria-label="Remove Root Key 3 Press binding"');
  });
});
