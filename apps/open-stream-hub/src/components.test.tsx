import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { CrashScreen, ErrorBoundary, IntegerField, LabelError, ModalDialog } from './controls';
import { KeypadScreen } from './KeypadScreen';
import { createKeypadFolder, keypadPagesAtPath, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import { BindingsDrawer } from './DevicesPage';
import { ProfileManagerSheet, SampleTestPanel, VersionHistory } from './ProfileManager';
import { browserProfile } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

// A session over a recorded invoke, as in the session tests.
function recordedSession() {
  const calls: { command: string; args?: Record<string, unknown>; resolve(value: unknown): void }[] = [];
  const session = new ProfileSession(<T,>(command: string, args?: Record<string, unknown>) => new Promise<T>(resolve => {
    calls.push({ command, args, resolve: resolve as (value: unknown) => void });
  }), browserProfile, true);
  return { session, calls };
}

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

describe('earlier versions panel', () => {
  it('lists the kept versions with restore buttons once loaded', () => {
    const { session } = recordedSession();
    const state = { ...session.getState(), draft: { ...session.getState().draft, profileId: 'a' }, versions: [{ modified_at_ms: 1_700_000_000_000 }], versionsId: 'a', versionsError: null };
    const markup = renderToStaticMarkup(<VersionHistory session={session} state={state} disabled={false} now={1_700_000_100_000} />);
    expect(markup).toContain('Earlier versions');
    expect(markup).toContain('Saved at');
    expect(markup).toContain('>Restore</button>');
    // Closed by default; nothing fetched during render.
    expect(markup).toContain('<details class="panel version-history"');
  });

  it('shows the empty state and a failed listing honestly', () => {
    const { session } = recordedSession();
    const base = { ...session.getState(), draft: { ...session.getState().draft, profileId: 'a' }, versionsId: 'a', versionsError: null };
    const empty = renderToStaticMarkup(<VersionHistory session={session} state={{ ...base, versions: [] }} disabled={false} now={0} />);
    expect(empty).toContain('No earlier versions yet');
    const failed = renderToStaticMarkup(<VersionHistory session={session} state={{ ...base, versions: null, versionsError: 'Could not list the earlier versions: gone' }} disabled={false} now={0} />);
    expect(failed).toContain('Could not list the earlier versions: gone');
    expect(failed).not.toContain('No earlier versions yet');
  });

  it('restores through the session when a Restore button is pressed', async () => {
    const { session, calls } = recordedSession();
    const boot = session.bootstrap();
    calls[0].resolve({ profile: { id: 'a', modified_at_ms: 1000, profile: upgradeKeypadProfile(browserProfile) }, profiles: [{ id: 'a', name: 'Everyday', binding_count: 4, modified_at_ms: 1000, dry_run_only: false }], unreadable: [], storage_path: '/s', desktop: true });
    await boot;
    const state = { ...session.getState(), draft: { ...session.getState().draft, profileId: 'a' }, versions: [{ modified_at_ms: 500 }], versionsId: 'a', versionsError: null };
    const panel = VersionHistory({ session, state, disabled: false, now: 1_000 });
    // Walk the returned element tree to the row's Restore handler.
    const list = panel.props.children.find((child: { props?: { className?: string } }) => child?.props?.className === 'version-list');
    const row = list.props.children[0];
    const button = row.props.children.find((child: { props?: { onClick?: () => void } }) => typeof child?.props?.onClick === 'function');
    button.props.onClick();
    await Promise.resolve();
    expect(calls.map(call => call.command)).toEqual(['bootstrap', 'load_profile_version']);
  });

  it('fetches the list when the panel opens for a profile with none loaded', () => {
    const { session, calls } = recordedSession();
    const state = { ...session.getState(), draft: { ...session.getState().draft, profileId: 'a' }, versions: null, versionsId: null, versionsError: null };
    const panel = VersionHistory({ session, state, disabled: false, now: 0 });
    const toggle = panel.props.onToggle;
    toggle({ currentTarget: { open: true } });
    expect(calls.map(call => call.command)).toEqual(['profile_versions']);
    // Opening again asks again, so versions kept while it was closed appear.
    const listed = { ...state, versions: [{ modified_at_ms: 1 }], versionsId: 'a' };
    VersionHistory({ session, state: listed, disabled: false, now: 0 }).props.onToggle({ currentTarget: { open: true } });
    expect(calls).toHaveLength(2);
    // Closing does not fetch.
    VersionHistory({ session, state: listed, disabled: false, now: 0 }).props.onToggle({ currentTarget: { open: false } });
    expect(calls).toHaveLength(2);
  });
});

describe('profile manager sheet', () => {
  const sheet = (session: ProfileSession, state = session.getState()) =>
    renderToStaticMarkup(<ProfileManagerSheet session={session} state={state} desktop={false} disabled confirm={() => {}} onAllow={() => {}} onClose={() => {}} />);

  it('lists unreadable profiles with the storage folder and offers delete for saved profiles', () => {
    const session = new ProfileSession(() => new Promise(() => {}), browserProfile, false);
    const base = session.getState();
    const state = {
      ...base, storagePath: '/store/profiles',
      profiles: [{ id: 'a', name: 'Alpha', binding_count: 2, modified_at_ms: 1, dry_run_only: false }],
      unreadable: [{ id: 'bad', error: 'invalid profile JSON' }],
    };
    const markup = sheet(session, state);
    expect(markup).toContain('Unreadable profiles');
    expect(markup).toContain('bad.json');
    expect(markup).toContain('invalid profile JSON');
    expect(markup).toContain('/store/profiles');
    expect(markup).toContain('aria-label="Delete profile Alpha"');
  });

  it('keeps the open profile, the saved list, import and export, and the focused-app rules in one sheet', () => {
    const session = new ProfileSession(() => new Promise(() => {}), browserProfile, false);
    const markup = sheet(session);
    expect(markup).toContain('aria-label="Profiles"');
    expect(markup).toContain('id="profile-name"');
    expect(markup).toContain('Validate changes');
    expect(markup).toContain('Earlier versions');
    expect(markup).toContain('New profile');
    expect(markup).toContain('Import official .mhz');
    expect(markup).toContain('Focused-app rules');
  });

  it('offers Remove for keypad bindings from the bindings drawer', () => {
    const session = new ProfileSession(() => new Promise(() => {}), { schema_version: 1, name: 'Keys', bindings: [{ module: 'm4315', control: 2, gesture: 'pressed', action: { type: 'toggle_mute' } }] }, false);
    const markup = renderToStaticMarkup(<BindingsDrawer open onToggle={() => {}} profile={session.getState().draft.profile} session={session} editor={{} as DeviceEditor} disabled />);
    expect(markup).toContain('aria-label="Remove Root Key 3 Press binding"');
  });
});

describe('bindings drawer', () => {
  const session = new ProfileSession(() => new Promise(() => {}), { schema_version: 1, name: 'K', bindings: [{ module: 'm003k', control: 0, gesture: 'clockwise', action: { type: 'volume', delta: 5 } }] }, false);
  const drawer = (open: boolean) => renderToStaticMarkup(<BindingsDrawer open={open} onToggle={() => {}} profile={session.getState().draft.profile} session={session} editor={{} as DeviceEditor} disabled />);

  it('is collapsed to a count by default and expands to the full list', () => {
    const closed = drawer(false);
    expect(closed).toContain('aria-expanded="false"');
    expect(closed).toContain('aria-controls="bindings-drawer-content"');
    expect(closed).toContain('1 in this profile');
    expect(closed).not.toContain('binding-search');
    const open = drawer(true);
    expect(open).toContain('aria-expanded="true"');
    expect(open).toContain('id="bindings-drawer-content"');
    expect(open).toContain('Search bindings');
    expect(open).toContain('Knob 1 · Clockwise');
  });

  it('shows a warning pill when the profile has binding warnings', () => {
    const warned = new ProfileSession(() => new Promise(() => {}), { schema_version: 1, name: 'W', bindings: [
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
      { module: 'm003k', control: 0, gesture: 'released', action: { type: 'toggle_mute' } },
    ] }, false);
    // A release with no press gesture bound is not a warning on knobs; a duplicate selector is.
    warned.update(latest => ({ ...latest, bindings: [latest.bindings[0], { ...latest.bindings[0] }] }));
    const markup = renderToStaticMarkup(<BindingsDrawer open={false} onToggle={() => {}} profile={warned.getState().draft.profile} session={warned} editor={{} as DeviceEditor} disabled />);
    expect(markup).toContain('drawer-warnings');
    expect(markup).toContain('aria-label="All bindings in this profile"');
  });
});

describe('sample test panel', () => {
  it('keeps the sample-input test as a collapsed section with its summary visible', () => {
    const session = new ProfileSession(() => new Promise(() => {}), browserProfile, false);
    const markup = renderToStaticMarkup(<SampleTestPanel session={session} state={session.getState()} disabled replayKind="knobs" setReplayKind={() => {}} />);
    expect(markup).toContain('<details class="panel replay-panel">');
    expect(markup).toContain('<summary class="replay-summary"><span>Test with sample input</span><span class="small-label">No hardware required</span></summary>');
    expect(markup).toContain('>Run test</button>');
  });
});
