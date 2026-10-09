import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ActionFields } from './ActionFields';
import type { BindingAction, StepAction } from './contracts';

const render = (action: BindingAction | null, fader = false) =>
  renderToStaticMarkup(<ActionFields action={action} fader={fader} selectorKey="m4315:root:0:pressed:" onChange={() => {}} />);

function ids(markup: string) {
  return [...markup.matchAll(/ id="([^"]+)"/g)].map(match => match[1]);
}

// Every id is unique and every label points at an existing control.
function expectAccessible(markup: string) {
  const all = ids(markup);
  expect(new Set(all).size).toBe(all.length);
  for (const [, target] of markup.matchAll(/<label for="([^"]+)"/g)) expect(all).toContain(target);
}

describe('action fields', () => {
  it('offers only the position action for faders and the full catalogue elsewhere', () => {
    const fader = render(null, true);
    expect(fader).toContain('value="volume_from_position"');
    expect(fader).not.toContain('value="open_url"');
    const key = render(null);
    for (const type of ['open_url', 'text', 'media', 'system', 'multi_action', 'shortcut', 'profile']) expect(key).toContain(`value="${type}"`);
    expect(key).not.toContain('value="volume_from_position"');
    expect(fader).not.toContain('value="profile"');
  });

  it('keeps the historical top-level field ids', () => {
    expect(ids(render({ type: 'volume', delta: 5 }))).toContain('volume-delta');
    expect(ids(render({ type: 'launch_app', bundle_id: 'com.apple.finder' }))).toContain('app-bundle-id');
    expect(ids(render({ type: 'shortcut', key: 'c', modifiers: ['command'] }))).toContain('shortcut-key');
  });

  it('shows a named-key select without the character input', () => {
    const markup = render({ type: 'shortcut', key: 'arrow_up', modifiers: [] });
    expect(markup).toMatch(/<option value="arrow_up" selected="">Up Arrow<\/option>/);
    expect(ids(markup)).not.toContain('shortcut-key');
    expectAccessible(markup);
  });

  it('renders URL, text, media and system fields with inline problems', () => {
    const url = render({ type: 'open_url', url: 'file:///etc/passwd' });
    expect(url).toContain('aria-invalid="true"');
    expect(url).toContain('role="alert"');
    expectAccessible(url);
    const text = render({ type: 'text', text: 'hi', press_enter: true });
    expect(text).toContain('<textarea');
    expect(text).toContain('Press Return after pasting');
    expect(text).not.toContain('role="alert"');
    expectAccessible(text);
    const stop = render({ type: 'media', command: 'stop' });
    expect(stop).toContain('Apple Music');
    expect(stop).not.toContain('Preview only');
    expect(stop).toContain('does not start a player that is not running');
    expect(stop).not.toContain('never opens');
    expect(render({ type: 'media', command: 'play_pause' })).toContain('Accessibility');
    expect(render({ type: 'system', command: 'screenshot' })).toContain('Screen Recording');
  });

  it('renders the profile direction select without offering it as a multi-action step', () => {
    const markup = render({ type: 'profile', direction: 'previous' });
    expect(markup).toMatch(/<option value="previous" selected="">Previous profile<\/option>/);
    expect(markup).toContain('value="next"');
    expect(markup).toContain('pause until you confirm the new profile');
    expect(markup).toContain('Only runs started from this app switch');
    expect(markup).toContain('Learn more');
    expectAccessible(markup);
    const steps = render({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'toggle_mute' }] });
    expect(steps).not.toMatch(/id="step-1-action"[^]*value="profile"[^]*id="step-2-action"/);
    expect(steps.match(/value="profile"/g)).toHaveLength(1);
  });

  it('offers timers on keys and dials only, never on faders or as a multi-action step', () => {
    expect(render(null)).toContain('value="timer"');
    expect(render(null, true)).not.toContain('value="timer"');
    const steps = render({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'toggle_mute' }] });
    expect(steps.match(/value="timer"/g)).toHaveLength(1);
  });

  it('renders timer duration, optional label and the closed sound list', () => {
    const markup = render({ type: 'timer', seconds: 90 });
    expect(markup).toMatch(/<option value="timer" selected="">Timer<\/option>/);
    expect(ids(markup)).toEqual(expect.arrayContaining(['timer-seconds', 'timer-label', 'timer-sound']));
    expect(markup).toMatch(/id="timer-seconds"[^>]*min="1"[^>]*max="86400"/);
    // Absent sound shows the service default.
    expect(markup).toMatch(/<option value="glass" selected="">Glass<\/option>/);
    expect(markup.match(/<option value="[a-z]+"[^>]*>/g)?.filter(option => /basso|blow|bottle|frog|funk|glass|hero|morse|ping|pop|purr|sosumi|submarine|tink|none/.test(option))).toHaveLength(15);
    expect(markup).toContain('not shown on the key');
    expect(markup).toContain('pressing the same control again cancels it');
    expect(markup).not.toContain('role="alert"');
    expectAccessible(markup);
    const labelled = render({ type: 'timer', seconds: 5, label: 'Tea', sound: 'none' });
    expect(labelled).toContain('value="Tea"');
    expect(labelled).toMatch(/<option value="none" selected="">No sound<\/option>/);
    const bad = render({ type: 'timer', seconds: 5, label: ' padded' });
    expect(bad).toContain('aria-invalid="true"');
    expect(bad).toContain('id="timer-label-error"');
    expectAccessible(bad);
  });

  it('renders ordered multi-action steps with unique ids and bounded add/remove', () => {
    const steps: StepAction[] = [{ type: 'volume', delta: 5 }, { type: 'shortcut', key: 'tab', modifiers: ['command'] }];
    const two = render({ type: 'multi_action', actions: steps });
    expectAccessible(two);
    expect(ids(two)).toEqual(expect.arrayContaining(['step-1-action', 'step-1-volume-delta', 'step-2-action', 'step-2-shortcut-key-kind']));
    // Only the top-level select offers multi_action; steps cannot nest.
    expect(two.match(/value="multi_action"/g)).toHaveLength(1);
    expect(two.match(/aria-label="Remove step \d"/g)).toHaveLength(2);
    expect(two).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Remove step 1"/);
    const eight = render({ type: 'multi_action', actions: Array.from({ length: 8 }, () => ({ type: 'toggle_mute' as const })) });
    expect(eight).toMatch(/<button[^>]*disabled=""[^>]*>.*Add step<\/button>/);
    expect(eight).not.toMatch(/<button[^>]*disabled=""[^>]*aria-label="Remove step 1"/);
  });
});

describe('obs_scene fields', () => {
  it('edits the scene name with inline problems and explains what it needs', () => {
    const markup = render({ type: 'obs_scene', scene: 'Live' });
    expect(markup).toContain('<label for="obs-scene">OBS scene name</label>');
    expect(markup).toContain('value="Live"');
    expect(markup).not.toContain('role="alert"');
    expect(markup).toContain('OBS turned on in Settings');
    expectAccessible(markup);
    const blank = render({ type: 'obs_scene', scene: ' ' });
    expect(blank).toContain('aria-invalid="true"');
    expect(blank).toContain('role="alert"');
    expect(render(null)).toContain('value="obs_scene"');
    expect(render(null, true)).not.toContain('value="obs_scene"');
  });

  it('is offered as a multi-action step with prefixed ids', () => {
    const markup = render({ type: 'multi_action', actions: [{ type: 'obs_scene', scene: 'Live' }, { type: 'toggle_mute' }] });
    expect(ids(markup)).toContain('step-1-obs-scene');
    expect(markup).toMatch(/<option value="obs_scene" selected="">Switch OBS scene<\/option>/);
    expectAccessible(markup);
  });
});

describe('obs output fields', () => {
  it('edits the virtual camera and replay buffer operation from closed lists', () => {
    const camera = render({ type: 'obs_virtual_camera', op: 'start' });
    expect(camera).toContain('<label for="obs-virtual-camera-op">Virtual camera</label>');
    expect(camera).toMatch(/<option value="start" selected="">Start<\/option>/);
    expect(camera).not.toContain('value="save"');
    expect(camera).toContain('OBS turned on in Settings');
    expectAccessible(camera);
    const buffer = render({ type: 'obs_replay_buffer', op: 'save' });
    expect(buffer).toContain('<label for="obs-replay-buffer-op">Replay buffer</label>');
    expect(buffer).toMatch(/<option value="save" selected="">Save the last seconds<\/option>/);
    expect(buffer).toContain('Saving needs the buffer to be running');
    expectAccessible(buffer);
    for (const type of ['obs_virtual_camera', 'obs_replay_buffer']) {
      expect(render(null)).toContain(`value="${type}"`);
      expect(render(null, true)).not.toContain(`value="${type}"`);
    }
  });

  it('is offered as a multi-action step with prefixed ids', () => {
    const markup = render({ type: 'multi_action', actions: [{ type: 'obs_replay_buffer', op: 'save' }, { type: 'obs_virtual_camera', op: 'stop' }] });
    expect(ids(markup)).toEqual(expect.arrayContaining(['step-1-obs-replay-buffer-op', 'step-2-obs-virtual-camera-op']));
    expectAccessible(markup);
  });
});

describe('obs source fields', () => {
  it('edits studio mode and the transition', () => {
    const studio = render({ type: 'obs_studio_mode', op: 'on' });
    expect(studio).toContain('<label for="obs-studio-mode-op">Studio mode</label>');
    expect(studio).toMatch(/<option value="on" selected="">Turn on<\/option>/);
    expectAccessible(studio);
    const transition = render({ type: 'obs_transition' });
    expect(transition).toContain('studio-mode transition');
    expect(transition).toContain('OBS turned on in Settings');
    expectAccessible(transition);
  });

  it('edits both names of a filter with inline problems', () => {
    const markup = render({ type: 'obs_filter', source: 'Webcam', filter: 'Blur', op: 'toggle' });
    expect(markup).toContain('<label for="obs-filter-source">Source name</label>');
    expect(markup).toContain('for="obs-filter-name">Filter name</label>');
    expect(markup).not.toContain('role="alert"');
    expectAccessible(markup);
    const blank = render({ type: 'obs_filter', source: 'Webcam', filter: ' ', op: 'toggle' });
    expect(blank).toContain('OBS filter name');
    expect(blank).toContain('role="alert"');
  });

  it('edits the name of an OBS scene collection or profile and warns that OBS reloads', () => {
    const markup = render({ type: 'obs_scene_collection', name: 'Show' });
    expect(markup).toContain('<label for="obs-reload-name">OBS scene collection name</label>');
    expect(markup).toContain('reloads');
    expect(markup).toContain('put it last');
    expect(markup).not.toContain('role="alert"');
    expectAccessible(markup);
    const blank = render({ type: 'obs_profile', name: ' ' });
    expect(blank).toContain('>OBS profile name</label>');
    expect(blank).toContain('Enter the OBS profile name');
    const multi = render({ type: 'multi_action', actions: [{ type: 'obs_profile', name: 'A' }, { type: 'obs_scene_collection', name: 'B' }] });
    expect(ids(multi)).toEqual(expect.arrayContaining(['step-1-obs-reload-name', 'step-2-obs-reload-name']));
    expectAccessible(multi);
  });

  it('edits scene and source for visibility, also as a multi-action step', () => {
    const markup = render({ type: 'obs_source_visibility', scene: 'Live', source: '', op: 'off' });
    expect(markup).toContain('for="obs-visibility-scene">Scene name</label>');
    expect(markup).toContain('for="obs-visibility-source">Source name</label>');
    expect(markup).toContain('OBS source name');
    expectAccessible(markup);
    const multi = render({ type: 'multi_action', actions: [{ type: 'obs_studio_mode', op: 'on' }, { type: 'obs_filter', source: 'A', filter: 'B', op: 'off' }] });
    expect(ids(multi)).toEqual(expect.arrayContaining(['step-1-obs-studio-mode-op', 'step-2-obs-filter-source', 'step-2-obs-filter-name', 'step-2-obs-filter-op']));
    expectAccessible(multi);
    for (const type of ['obs_studio_mode', 'obs_transition', 'obs_filter', 'obs_source_visibility']) expect(render(null)).toContain(`value="${type}"`);
  });
});
