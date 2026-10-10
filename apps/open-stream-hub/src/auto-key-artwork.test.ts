import { describe, expect, it } from 'vitest';
import type { KeyArtwork, Profile } from './contracts';
import { applyAutoArtwork, autoDesign, buildAutoArtwork, isAutoDrawn, mayReplaceArtwork, planAutoArtwork, planKey, urlHost } from './auto-key-artwork';
import type { AutoDeps, AutoPlan } from './auto-key-artwork';
import { appIconId, storedKeyArtworkDesign } from './key-artwork-design';
import { emptyKeypadPage } from './keypad-model';

const profile = (): Profile => ({ schema_version: 2, name: 'Art', bindings: [], keypad: { pages: [emptyKeypadPage()] } });
const prepared: KeyArtwork = { frames: [{ jpeg_base64: 'AAAA', delay_ms: 100 }], loop_count: 0 };
const finder: AutoPlan = { kind: 'app', bundleId: 'com.apple.finder' };

function deps(overrides: Partial<AutoDeps> = {}) {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const rendered: { iconId: string | null; appIcon?: string }[] = [];
  const made: AutoDeps = {
    invoke: (async (command: string, args?: Record<string, unknown>) => { calls.push({ command, args }); return prepared; }) as AutoDeps['invoke'],
    render: async (design, appIcon) => { rendered.push({ iconId: design.iconId, appIcon }); return 'UE5H'; },
    appIcon: async () => 'SUNPTg==',
    ...overrides,
  };
  return { made, calls, rendered };
}

describe('planning the automatic artwork', () => {
  it('plans an application icon or a site label, and nothing else', () => {
    expect(planAutoArtwork({ type: 'launch_app', bundle_id: ' com.apple.finder ' })).toEqual(finder);
    expect(planAutoArtwork({ type: 'launch_app', bundle_id: '  ' })).toBeNull();
    expect(planAutoArtwork({ type: 'open_url', url: 'https://www.example.com/path?q=1' })).toEqual({ kind: 'url', host: 'example.com' });
    expect(planAutoArtwork({ type: 'open_url', url: 'mailto:a@b.c' })).toBeNull();
    expect(planAutoArtwork({ type: 'open_url', url: 'not a url' })).toBeNull();
    expect(planAutoArtwork({ type: 'toggle_mute' })).toBeNull();
    expect(planAutoArtwork(null)).toBeNull();
    expect(planKey(finder)).toBe('app:com.apple.finder');
    expect(planKey(null)).toBeNull();
  });

  it('keeps the host short and printable', () => {
    expect(urlHost('http://localhost:3000')).toBe('localhost');
    expect(urlHost('https://' + 'a'.repeat(200) + '.com')).toBeNull();
  });

  it('draws the app icon alone, and a globe with the host for a URL', () => {
    expect(autoDesign(finder)).toMatchObject({ iconId: appIconId('com.apple.finder'), text: '' });
    expect(autoDesign({ kind: 'url', host: 'example.com' })).toMatchObject({ iconId: 'globe', text: 'example.com', textSize: 0 });
  });
});

describe('building the artwork', () => {
  it('renders with the application icon and stores the design beside the frames', async () => {
    const { made, calls, rendered } = deps();
    const artwork = await buildAutoArtwork(finder, made);
    expect(rendered).toEqual([{ iconId: appIconId('com.apple.finder'), appIcon: 'SUNPTg==' }]);
    expect(calls).toHaveLength(1);
    expect(calls[0].command).toBe('prepare_key_artwork');
    expect(calls[0].args).toEqual({ request: expect.objectContaining({ image_base64: 'UE5H', target: 'keypad', background: [0, 0, 0] }) });
    expect(artwork.frames).toEqual(prepared.frames);
    expect(artwork.design).toEqual(storedKeyArtworkDesign(autoDesign(finder)));
  });

  it('needs no application icon for a URL, and fails when the icon cannot be read', async () => {
    const { made, rendered } = deps({ appIcon: async () => { throw new Error('No app'); } });
    await buildAutoArtwork({ kind: 'url', host: 'example.com' }, made);
    expect(rendered[0].appIcon).toBeUndefined();
    await expect(buildAutoArtwork(finder, made)).rejects.toThrow('No app');
  });
});

describe('what may be replaced', () => {
  const drawn = (plan: AutoPlan): KeyArtwork => ({ ...prepared, design: storedKeyArtworkDesign(autoDesign(plan)) });
  it('replaces no artwork or an earlier automatic drawing, and never a designed or uploaded one', () => {
    expect(mayReplaceArtwork(undefined)).toBe(true);
    expect(mayReplaceArtwork(drawn(finder))).toBe(true);
    expect(mayReplaceArtwork(drawn({ kind: 'url', host: 'example.com' }))).toBe(true);
    // An upload carries no design.
    expect(mayReplaceArtwork(prepared)).toBe(false);
    // A drawing the user changed in any field is theirs.
    const edited = drawn(finder);
    expect(mayReplaceArtwork({ ...edited, design: { ...edited.design!, text: 'Files' } })).toBe(false);
    expect(mayReplaceArtwork({ ...edited, design: { ...edited.design!, background: '#112233' } })).toBe(false);
    expect(mayReplaceArtwork({ ...edited, design: { ...edited.design!, icon_id: 'clapperboard' } })).toBe(false);
    expect(isAutoDrawn(undefined)).toBe(false);
  });
});

describe('applying it to the key', () => {
  function setup(artwork?: KeyArtwork, extra: Partial<AutoDeps> = {}) {
    const state = { draftGeneration: 1, draft: { profile: profile() } };
    const edits: { profile: Profile; key?: string }[] = [];
    const session = { getState: () => state, edit: (next: Profile, key?: string) => { edits.push({ profile: next, key }); state.draft.profile = next; } };
    let current = true;
    let planId: string | null = planKey(finder);
    let present = artwork;
    const context = { planKey: () => planId, artwork: () => present, current: () => current };
    const { made } = deps(extra);
    return { state, edits, session, context, made, supersede: () => { current = false; }, retarget: (id: string | null) => { planId = id; }, setArtwork: (value: KeyArtwork | undefined) => { present = value; } };
  }

  it('sets the artwork as one undoable edit on the key', async () => {
    const { session, context, made, edits, state } = setup();
    expect(await applyAutoArtwork(finder, made, session, ['root'], 2, context)).toBe(true);
    expect(edits).toHaveLength(1);
    expect(edits[0].key).toBe('key-artwork-auto');
    expect(state.draft.profile.keypad!.pages[0].artwork!['2'].design).toEqual(storedKeyArtworkDesign(autoDesign(finder)));
  });

  it('leaves designed artwork alone, before and after drawing', async () => {
    const mine = { ...prepared, design: { ...storedKeyArtworkDesign(autoDesign(finder)), text: 'Mine' } };
    const first = setup(mine);
    expect(await applyAutoArtwork(finder, first.made, first.session, ['root'], 0, first.context)).toBe(false);
    expect(first.edits).toHaveLength(0);
    // Designed while the drawing was being prepared.
    const second = setup(undefined, { render: async () => { second.setArtwork(mine); return 'UE5H'; } });
    expect(await applyAutoArtwork(finder, second.made, second.session, ['root'], 0, second.context)).toBe(false);
    expect(second.edits).toHaveLength(0);
  });

  it('drops a drawing that was superseded, retargeted or met a replaced draft', async () => {
    const superseded = setup(undefined, { appIcon: async () => { superseded.supersede(); return 'SUNPTg=='; } });
    expect(await applyAutoArtwork(finder, superseded.made, superseded.session, ['root'], 0, superseded.context)).toBe(false);
    const retargeted = setup(undefined, { appIcon: async () => { retargeted.retarget('app:com.apple.mail'); return 'SUNPTg=='; } });
    expect(await applyAutoArtwork(finder, retargeted.made, retargeted.session, ['root'], 0, retargeted.context)).toBe(false);
    const replaced = setup(undefined, { appIcon: async () => { replaced.state.draftGeneration += 1; return 'SUNPTg=='; } });
    expect(await applyAutoArtwork(finder, replaced.made, replaced.session, ['root'], 0, replaced.context)).toBe(false);
    for (const attempt of [superseded, retargeted, replaced]) expect(attempt.edits).toHaveLength(0);
  });

  it('leaves the key as it is when the icon cannot be read or prepared', async () => {
    const unreadable = setup(undefined, { appIcon: async () => { throw new Error('No app'); } });
    expect(await applyAutoArtwork(finder, unreadable.made, unreadable.session, ['root'], 0, unreadable.context)).toBe(false);
    const failing = setup(undefined, { invoke: (async () => { throw new Error('boom'); }) as AutoDeps['invoke'] });
    expect(await applyAutoArtwork(finder, failing.made, failing.session, ['root'], 0, failing.context)).toBe(false);
    expect(unreadable.edits.length + failing.edits.length).toBe(0);
  });

  it('follows the application: its own earlier drawing is replaced', async () => {
    const mail: AutoPlan = { kind: 'app', bundleId: 'com.apple.mail' };
    const earlier = { ...prepared, design: storedKeyArtworkDesign(autoDesign(finder)) };
    const next = setup(earlier);
    next.retarget(planKey(mail));
    expect(await applyAutoArtwork(mail, next.made, next.session, ['root'], 0, next.context)).toBe(true);
    expect(next.state.draft.profile.keypad!.pages[0].artwork!['0'].design!.icon_id).toBe(appIconId('com.apple.mail'));
  });
});
