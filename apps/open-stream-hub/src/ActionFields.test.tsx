import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ActionFields, ObsFields } from './ActionFields';
import { obsOperationLabels } from './action-model';
import type { ObsStepAction } from './action-model';
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

// Direct element-tree helpers: ObsFields is hook-free, so tests call it as a function and
// invoke the handlers exactly as the browser would.
type AnyElement = ReactElement<{ [key: string]: unknown }>;

function elementsOf(node: ReactNode): AnyElement[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(elementsOf);
  if (isValidElement(node)) return [node as AnyElement, ...elementsOf((node.props as { children?: ReactNode }).children)];
  return [];
}

describe('action fields', () => {
  it('replaces the flat select with the picker trigger', () => {
    const key = render(null);
    expect(key).toContain('<label for="binding-action">Action</label>');
    expect(key).toMatch(/<button[^>]*id="binding-action"[^>]*aria-haspopup="dialog"[^>]*aria-expanded="false"/);
    expect(key).toContain('Choose an action');
    // The closed picker carries no catalogue; its entries are covered by the action-picker tests.
    expect(key).not.toContain('data-action-type=');
    expectAccessible(key);
    const fader = render(null, true);
    expect(fader).toMatch(/id="binding-action"[^>]*aria-haspopup="dialog"/);
    const bound = render({ type: 'volume_from_position' }, true);
    expect(bound).toContain('>Output volume follows position</span>');
    expect(bound).toContain('Each position change plans an absolute output volume');
    expect(bound).toContain('aria-expanded="false"');
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
    expect(markup).toContain('>Switch profile</span>');
    expectAccessible(markup);
    const steps = render({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'toggle_mute' }] });
    expect(steps).not.toContain('>Switch profile</span>');
  });

  it('offers timers on keys and dials only, never on faders or as a multi-action step', () => {
    expect(render({ type: 'timer', seconds: 90 })).toContain('>Timer</span>');
    // Catalogue validity (faders, steps) is covered by the action-picker-model tests.
    const steps = render({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'toggle_mute' }] });
    expect(steps).not.toContain('>Timer</span>');
  });

  it('renders timer duration, optional label and the closed sound list', () => {
    const markup = render({ type: 'timer', seconds: 90 });
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
    // Each step shows its action on its own picker trigger.
    expect(two).toContain('>Change output volume</span>');
    expect(two).toContain('>Keyboard shortcut</span>');
    expect(two.match(/aria-label="Remove step \d"/g)).toHaveLength(2);
    expect(two).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Remove step 1"/);
    const eight = render({ type: 'multi_action', actions: Array.from({ length: 8 }, () => ({ type: 'toggle_mute' as const })) });
    expect(eight).toMatch(/<button[^>]*disabled=""[^>]*>.*Add step<\/button>/);
    expect(eight).not.toMatch(/<button[^>]*disabled=""[^>]*aria-label="Remove step 1"/);
  });
});

describe('obs fields', () => {
  it('edits every operation from one editor with an Operation select and one connection status', () => {
    const markup = render({ type: 'obs_scene', scene: 'Live' });
    expect(markup).toContain('<label for="obs-operation">Operation</label>');
    expect(markup).toMatch(/<option value="obs_scene" selected="">Scene<\/option>/);
    expect(markup).toContain('<label for="obs-scene">OBS scene name</label>');
    expect(markup).toContain('value="Live"');
    expect(markup).toContain('>Switch OBS scene</span>');
    // One connection status per editor, with the Set up link to Settings.
    expect(markup.match(/WebSocket server on/g)).toHaveLength(1);
    expect(markup.match(/Set up in Settings/g)).toHaveLength(1);
    expect(markup).not.toContain('OBS turned on in Settings');
    expectAccessible(markup);
    const blank = render({ type: 'obs_scene', scene: ' ' });
    expect(blank).toContain('aria-invalid="true"');
    expect(blank).toContain('role="alert"');
    expectAccessible(blank);
  });

  it('offers the nine operations in the Operation select', () => {
    const markup = render({ type: 'obs_transition' });
    for (const [type, label] of Object.entries(obsOperationLabels)) expect(markup).toMatch(new RegExp(`<option value="${type}"( selected="")?>${label}</option>`));
    expect(markup).toMatch(/<option value="obs_transition" selected="">Studio transition<\/option>/);
    // The transition has no parameters of its own, only the shared status.
    expect(ids(markup)).toEqual(expect.arrayContaining(['obs-operation']));
    expect(markup).toContain('studio-mode transition');
    expectAccessible(markup);
  });

  it('keeps the historical parameter fields of every operation', () => {
    const camera = render({ type: 'obs_virtual_camera', op: 'start' });
    expect(camera).toContain('<label for="obs-virtual-camera-op">Virtual camera</label>');
    expect(camera).toMatch(/<option value="start" selected="">Start<\/option>/);
    expect(camera).not.toContain('value="save"');
    const buffer = render({ type: 'obs_replay_buffer', op: 'save' });
    expect(buffer).toContain('<label for="obs-replay-buffer-op">Replay buffer</label>');
    expect(buffer).toMatch(/<option value="save" selected="">Save the last seconds<\/option>/);
    expect(render({ type: 'obs_studio_mode', op: 'on' })).toMatch(/<option value="on" selected="">Turn on<\/option>/);
    expect(ids(render({ type: 'obs_filter', source: 'Webcam', filter: 'Blur', op: 'toggle' })))
      .toEqual(expect.arrayContaining(['obs-filter-source', 'obs-filter-name', 'obs-filter-op']));
    expect(ids(render({ type: 'obs_source_visibility', scene: 'Live', source: 'Cam', op: 'off' })))
      .toEqual(expect.arrayContaining(['obs-visibility-scene', 'obs-visibility-source', 'obs-visibility-op']));
    const collection = render({ type: 'obs_scene_collection', name: 'Show' });
    expect(collection).toContain('<label for="obs-reload-name">OBS scene collection name</label>');
    expect(render({ type: 'obs_profile', name: 'P' })).toContain('>OBS profile name</label>');
    expectAccessible(camera);
    expectAccessible(buffer);
    expectAccessible(collection);
  });

  it('is offered as a multi-action step with prefixed ids, one status per step editor', () => {
    const markup = render({ type: 'multi_action', actions: [
      { type: 'obs_scene', scene: 'Live' }, { type: 'obs_replay_buffer', op: 'save' },
    ] });
    expect(ids(markup)).toEqual(expect.arrayContaining(['step-1-obs-operation', 'step-1-obs-scene', 'step-2-obs-operation', 'step-2-obs-replay-buffer-op']));
    expect(markup).toContain('>Switch OBS scene</span>');
    expect(markup).toContain('>OBS replay buffer</span>');
    expect((markup.match(/Set up in Settings/g) ?? []).length).toBe(2);
    expectAccessible(markup);
    const named = render({ type: 'multi_action', actions: [
      { type: 'obs_profile', name: 'A' }, { type: 'obs_filter', source: 'A', filter: 'B', op: 'off' },
    ] });
    expect(ids(named)).toEqual(expect.arrayContaining(['step-1-obs-reload-name', 'step-2-obs-filter-source', 'step-2-obs-filter-name', 'step-2-obs-filter-op']));
    expectAccessible(named);
  });

  it('keeps the per-operation behaviour notes without the repeated connection story', () => {
    expect(render({ type: 'obs_replay_buffer', op: 'save' })).toContain('Saving needs the buffer to be running');
    expect(render({ type: 'obs_virtual_camera', op: 'toggle' })).toContain('OBS refuses Start on a running output');
    expect(render({ type: 'obs_scene_collection', name: 'Show' })).toContain('reloads');
    expect(render({ type: 'obs_scene_collection', name: 'Show' })).toContain('put it last');
    const blankProfile = render({ type: 'obs_profile', name: ' ' });
    expect(blankProfile).toContain('Enter the OBS profile name');
    const filter = render({ type: 'obs_filter', source: 'Webcam', filter: 'Blur', op: 'toggle' });
    expect(filter).toContain('matched exactly');
    expect(filter).toContain('Toggle follows OBS');
    expect(filter).not.toContain('role="alert"');
    const blank = render({ type: 'obs_filter', source: 'Webcam', filter: ' ', op: 'toggle' });
    expect(blank).toContain('OBS filter name');
    expect(blank).toContain('role="alert"');
  });

  it('edits scene and source for visibility', () => {
    const markup = render({ type: 'obs_source_visibility', scene: 'Live', source: '', op: 'off' });
    expect(markup).toContain('for="obs-visibility-scene">Scene name</label>');
    expect(markup).toContain('for="obs-visibility-source">Source name</label>');
    expect(markup).toContain('OBS source name');
    expectAccessible(markup);
  });

  it('switches operation from the Operation select, keeping the names both share', () => {
    const treeOf = (action: ObsStepAction, prefix = '') => {
      const result: { changed?: StepAction; field?: string; setUp?: boolean } = {};
      const tree = ObsFields({
        action, prefix,
        onChange: (changed, field) => { result.changed = changed; result.field = field; },
        onSetUpObs: () => { result.setUp = true; },
      }) as AnyElement;
      return { result, tree };
    };
    const selectOperation = (tree: AnyElement, next: string) => {
      const select = elementsOf(tree).find(element => element.props.id === 'obs-operation');
      expect(select, 'Operation select').toBeTruthy();
      (select!.props as { onChange(event: { target: { value: string } }): void }).onChange({ target: { value: next } });
    };

    const scene = treeOf({ type: 'obs_scene', scene: 'Live' });
    selectOperation(scene.tree, 'obs_source_visibility');
    expect(scene.result).toEqual({ changed: { type: 'obs_source_visibility', scene: 'Live', source: 'Source', op: 'toggle' }, field: 'obs-operation' });
    // Operation settings reset; names unrelated to the new operation are dropped.
    const filter = treeOf({ type: 'obs_filter', source: 'Cam', filter: 'Blur', op: 'off' });
    selectOperation(filter.tree, 'obs_scene');
    expect(filter.result).toEqual({ changed: { type: 'obs_scene', scene: 'Scene' }, field: 'obs-operation' });
    const profile = treeOf({ type: 'obs_profile', name: 'Streaming' });
    selectOperation(profile.tree, 'obs_scene_collection');
    expect(profile.result).toEqual({ changed: { type: 'obs_scene_collection', name: 'Streaming' }, field: 'obs-operation' });

    const setUp = treeOf({ type: 'obs_transition' }, 'step-1-');
    const button = elementsOf(setUp.tree).find(element => element.type === 'button' && element.props.className === 'obs-setup');
    expect(button, 'Set up button').toBeTruthy();
    (button!.props as { onClick(): void }).onClick();
    expect(setUp.result.setUp).toBe(true);
  });
});
