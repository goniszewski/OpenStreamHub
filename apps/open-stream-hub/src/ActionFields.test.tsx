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
