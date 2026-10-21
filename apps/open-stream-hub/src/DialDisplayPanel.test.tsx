import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { Profile } from './contracts';
import { setDialAnimation, setDialDisplayContent, setDialImage } from './dial-display-model';
import { DialDisplayPanel } from './DialDisplayPanel';
import { DialEditorTabs } from './Inspector';
import { ProfileSession } from './profile-session';
import type { Invoke } from './profile-session';

const flat: Profile = { schema_version: 1, name: 'P', bindings: [{ module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 2 } }] };
const clock: Profile = setDialDisplayContent(flat, 'clock');
const pictured: Profile = setDialImage(flat, 'AAAA');

const harness = (profile: Profile, invoke?: Invoke) => ({ session: new ProfileSession(invoke ?? (async () => undefined as never), profile, false), sessionInvoke: invoke });

const render = (profile: Profile, invoke?: Invoke) => {
  const { session } = harness(profile, invoke);
  return { html: renderToStaticMarkup(<DialDisplayPanel profile={profile} session={session} disabled={false} invoke={invoke} />), session };
};

describe('DialDisplayPanel markup', () => {
  it('offers comparing the versions of a GIF, on by default, only for a custom image', () => {
    const { html } = render(pictured);
    expect(html).toMatch(/<input type="checkbox" checked=""[^>]*\/>Compare versions of a GIF/);
    expect(html).toContain('which takes about three times as long');
    expect(render(clock).html).not.toContain('Compare versions of a GIF');
  });
  it('offers every dial face, defaulting to the function name', () => {
    const { html } = render(flat);
    expect(html).toContain('aria-labelledby="dial-display-heading"');
    expect(html).toMatch(/<select id="dial-display-mode"[^>]*>/);
    for (const label of ['Function name (default)', 'Digital clock (HH:MM)', 'Analog clock', 'CPU usage (%, macOS only)', 'Custom image', 'Nothing (keys only)']) {
      expect(html).toContain(`>${label}</option>`);
    }
    expect(html).toContain('<option value="function" selected="">');
    expect(html).toContain('aria-label="Preview of the dial screen face"');
    expect(html).toContain('<canvas width="240" height="240"');
  });
  it('keeps the profile choice selected and notes what it means', () => {
    expect(render(clock).html).toContain('<option value="clock" selected="">');
    expect(render(clock).html).toContain('The clock faces follow the time on this computer.');
    const { html } = render(pictured);
    expect(html).toContain('<option value="image" selected="">');
    expect(html).toContain('<img src="data:image/jpeg;base64,AAAA"');
    expect(html).toContain('aria-label="Custom dial image"');
    expect(html).toContain('Remove image');
    expect(html).toContain('choose the picture again');
    expect(html).not.toContain('id="dial-image-fit"');
  });
  it('offers fit, rotation and background again when the profile kept the source picture', () => {
    const kept = setDialImage(flat, 'AAAA', { image: 'BBBB', fit: 'contain', quarter_turns: 1, background: [255, 0, 0] });
    const { html } = render(kept);
    expect(html).toContain('id="dial-image-fit"');
    expect(html).toContain('<option value="contain" selected="">');
    expect(html).toContain('value="#ff0000"');
    expect(html).toContain('>90°<');
    expect(html).not.toContain('choose the picture again');
  });
  it('describes an animated picture and takes GIF files', () => {
    const frames = [{ jpeg_base64: 'AAAA', delay_ms: 200 }, { jpeg_base64: 'BBBB', delay_ms: 300 }];
    const { html } = render(setDialAnimation(flat, { frames, loop_count: 0 }));
    expect(html).toContain('Plays 2 frames, looping.');
    expect(html).toContain('<img src="data:image/jpeg;base64,AAAA"');
    expect(html).toMatch(/<input id="dial-display-image-file"[^>]*accept="[^"]*image\/gif/);
    expect(render(setDialAnimation(flat, { frames, loop_count: 1 })).html).toContain('Plays 2 frames once.');
    expect(render(setDialAnimation(flat, { frames, loop_count: 3 })).html).toContain('Plays 2 frames, 3 times.');
    expect(render(pictured).html).not.toContain('Plays ');
  });
  it('asks for CPU honestly and only on the CPU face', () => {
    const { html } = render(setDialDisplayContent(flat, 'cpu'));
    expect(html).toContain('CPU usage is measured on this Mac');
    expect(html).toContain('illustrative value');
    expect(render(clock).html).not.toContain('illustrative value');
  });
  it('asks for a picture on the image face and needs the desktop app to prepare it', () => {
    const { html } = render(setDialDisplayContent(flat, 'image'));
    expect(html).toMatch(/<input id="dial-display-image-file"[^>]*disabled=""/);
    expect(html).toContain('Preparing the dial picture needs the desktop app.');
    expect(html).toContain('cropped to fill the round screen');
  });
  it('offers the file input in the desktop app, still disabled while the profile is locked', () => {
    const { session } = harness(flat);
    const html = renderToStaticMarkup(<DialDisplayPanel profile={setDialDisplayContent(flat, 'image')} session={session} disabled invoke={async () => undefined as never} />);
    expect(html).toMatch(/<input id="dial-display-image-file"[^>]*disabled=""/);
    expect(html).not.toContain('needs the desktop app');
  });
});

// The tab wrapper from the inspector, rendered standalone with the Display tab open.
const renderTabs = (profile: Profile, tab: 'functions' | 'display') => {
  const { session } = harness(profile);
  return renderToStaticMarkup(
    <DialEditorTabs tab={tab} onTab={() => {}} profile={profile} session={session} state={session.getState()} disabled={false} />
  );
};

describe('DialEditorTabs markup', () => {
  it('wires the Functions and Display tabs as a tablist', () => {
    const html = renderTabs(flat, 'functions');
    expect(html).toContain('role="tablist"');
    expect(html).toMatch(/<button type="button" role="tab" id="dial-tab-functions" aria-selected="true"/);
    expect(html).toMatch(/id="dial-tab-display" aria-selected="false"/);
    expect(html).toContain('aria-labelledby="dial-functions-heading"');
    expect(html).not.toContain('dial-display-heading');
  });
  it('shows the dial screen editor on the Display tab', () => {
    const html = renderTabs(clock, 'display');
    expect(html).toMatch(/id="dial-tab-display" aria-selected="true"/);
    expect(html).toContain('aria-labelledby="dial-display-heading"');
    expect(html).not.toContain('dial-functions-heading');
  });
});
