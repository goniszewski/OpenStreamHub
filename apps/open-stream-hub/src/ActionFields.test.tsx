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
    expect(render({ type: 'media', command: 'stop' })).toContain('Preview only');
    expect(render({ type: 'system', command: 'screenshot' })).toContain('Screen Recording');
  });

  it('renders the profile direction select without offering it as a multi-action step', () => {
    const markup = render({ type: 'profile', direction: 'previous' });
    expect(markup).toMatch(/<option value="previous" selected="">Previous profile<\/option>/);
    expect(markup).toContain('value="next"');
    expect(markup).toContain('pause until you confirm the new profile');
    expect(markup).toContain('standalone service');
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
