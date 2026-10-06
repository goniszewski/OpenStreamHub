import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { Binding, Profile } from './contracts';
import { BindingsPanel, ControlWarnings } from './BindingsPanel';
import { editKeypadBinding, keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import { browserProfile } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

const mute = { type: 'toggle_mute' } as const;
const dial = (gesture: Binding['gesture']): Binding => ({ module: 'm131e', control: 0, gesture, action: mute });
const profileWith = (...bindings: Binding[]): Profile => upgradeKeypadProfile({ schema_version: 1, name: 'P', bindings });
const keys = (control: number, label: string) => editKeypadBinding(upgradeKeypadProfile({ schema_version: 1, name: 'K', bindings: [] }), [keypadRoot], control, 'pressed', '', mute, label);

describe('BindingsPanel markup', () => {
  const session = (profile: Profile) => new ProfileSession(() => new Promise(() => {}), profile, false);
  const render = (profile: Profile, initialQuery = '') => renderToStaticMarkup(
    <BindingsPanel profile={profile} session={session(profile)} editor={{} as DeviceEditor} disabled={false} initialQuery={initialQuery} />);

  it('renders the search box with a count and no clear button while empty', () => {
    const html = render(upgradeKeypadProfile(browserProfile));
    expect(html).toContain('role="search"');
    expect(html).toContain('for="binding-search"');
    expect(html).toContain('id="binding-search"');
    expect(html).toContain('type="search"');
    expect(html).toContain('4 bindings');
    expect(html).not.toContain('Clear search');
    expect(html).not.toContain('binding-warnings');
  });

  it('filters rows, shows the result count and a clear button', () => {
    const html = render(upgradeKeypadProfile(browserProfile), 'counter');
    expect(html).toContain('1 of 4 bindings');
    expect(html).toContain('Counter-clockwise');
    expect(html).not.toContain('Toggle output mute');
    expect(html).toContain('aria-label="Clear search"');
    expect(html).toContain('value="counter"');
  });

  it('shows an accessible empty state when nothing matches', () => {
    const html = render(upgradeKeypadProfile(browserProfile), 'zzz');
    expect(html).toContain('0 of 4 bindings');
    expect(html).toContain('role="status"');
    expect(html).toContain('No bindings match “zzz”.');
    expect(html).toContain('Clear search');
    expect(html).not.toContain('class="binding-row"');
  });

  it('keeps the first-run empty state for a profile without bindings, with no search box', () => {
    const html = render(profileWith());
    expect(html).toContain('No bindings yet.');
    expect(html).not.toContain('role="search"');
  });

  it('lists warnings with a Show button regardless of the search', () => {
    const html = render(profileWith(dial('released'), dial('short_press')), 'nomatch');
    expect(html).toContain('role="region" aria-label="Binding warnings"');
    expect(html).toContain('Release and Short press both fire');
    expect(html).toContain('>Show</button>');
  });

  it('keeps the Remove labels for keypad bindings', () => {
    expect(render(keys(2, 'A'))).toContain('aria-label="Remove Root Key 3 Press binding"');
  });

  it('renders the inspector note only for an involved control', () => {
    const profile = profileWith(dial('released'), dial('short_press'));
    const editor = { selection: { module: 'm131e', control: 0, gesture: 'released', serial: '' } } as unknown as DeviceEditor;
    const note = renderToStaticMarkup(<ControlWarnings profile={profile} editor={editor} selected={{ module: 'm131e', control: 0 }} />);
    expect(note).toContain('Binding warnings');
    expect(note).toContain('>Show</button>');
    expect(renderToStaticMarkup(<ControlWarnings profile={profile} editor={editor} selected={{ module: 'm003k', control: 0 }} />)).toBe('');
  });
});
