import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ActionFields, ObsCombo, ObsConnection, ObsFields } from './ActionFields';
import { obsOperationLabels } from './action-model';
import type { ObsStepAction } from './action-model';
import type { ObsCatalogView, ObsNameKind } from './obs-catalog';
import type { BindingAction, ObsNameList, StepAction } from './contracts';

const render = (action: BindingAction | null, fader = false, onSetUpObs: (() => void) | null = () => {}) =>
  renderToStaticMarkup(<ActionFields action={action} fader={fader} selectorKey="m4315:root:0:pressed:" onChange={() => {}} onSetUpObs={onSetUpObs ?? undefined} />);

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

  it('replaces the key selects with the capture field and the key chooser', () => {
    const markup = render({ type: 'shortcut', key: 'arrow_up', modifiers: [] });
    // The capture input shows the chord and offers pressing keys; the chooser sits beside it.
    expect(markup).toContain('>Up Arrow</span>');
    expect(markup).toContain('placeholder="Press keys…"');
    expect(markup).toMatch(/aria-haspopup="dialog"[^>]*aria-expanded="false"/);
    expect(markup).toContain('A chord holds at most 5 keys');
    // The manual key select and character input are gone.
    expect(ids(markup)).not.toContain('shortcut-key-kind');
    expectAccessible(markup);
    const media = render({ type: 'shortcut', key: 'play_pause', modifiers: [] });
    expect(media).toContain('Play/Pause media key');
    expect(media).toContain('cannot combine with modifiers');
    expectAccessible(media);
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
    expect(ids(two)).toEqual(expect.arrayContaining(['step-1-action', 'step-1-volume-delta', 'step-2-action', 'step-2-shortcut-key']));
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
    expect(markup).toContain('>OBS</span>');
    expect(markup).not.toContain('Switch OBS scene');
    // One connection status per editor, with the Set up link to Settings.
    expect(markup.match(/WebSocket server on/g)).toHaveLength(1);
    expect(markup.match(/Set up in Settings/g)).toHaveLength(1);
    expect(markup).not.toContain('OBS turned on in Settings');
    // Without a handler the button is not rendered; the status text stays.
    const bare = render({ type: 'obs_scene', scene: 'Live' }, false, null);
    expect(bare).toContain('WebSocket server on');
    expect(bare).not.toContain('Set up in Settings');
    expect((bare.match(/obs-setup/g) ?? []).length).toBe(0);
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
    expect(camera).toContain('<label for="obs-virtual-camera-op">Action</label>');
    expect(camera).toMatch(/<option value="start" selected="">Start<\/option>/);
    expect(camera).not.toContain('value="save"');
    const buffer = render({ type: 'obs_replay_buffer', op: 'save' });
    expect(buffer).toContain('<label for="obs-replay-buffer-op">Action</label>');
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

  it('is offered as a multi-action step with prefixed ids and one status for the whole editor', () => {
    const markup = render({ type: 'multi_action', actions: [
      { type: 'obs_scene', scene: 'Live' }, { type: 'obs_replay_buffer', op: 'save' },
    ] });
    expect(ids(markup)).toEqual(expect.arrayContaining(['step-1-obs-operation', 'step-1-obs-scene', 'step-2-obs-operation', 'step-2-obs-replay-buffer-op']));
    expect(markup).toContain('>OBS</span>');
    expect((markup.match(/Set up in Settings/g) ?? []).length).toBe(1);
    expect((markup.match(/obs-connection/g) ?? []).length).toBe(1);
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

    // The connection status shows in the standalone editor and once per multi-action,
    // never per step; the Set up button renders only with a handler.
    const setUp = render({ type: 'obs_transition' });
    expect(setUp).toContain('class="obs-setup"');
    const inStep = renderToStaticMarkup(ObsFields({ action: { type: 'obs_transition' }, prefix: 'step-1-', onChange: () => {}, onSetUpObs: () => {} }));
    expect(inStep).not.toContain('WebSocket server on');
    expect(inStep).not.toContain('class="obs-setup"');

    let setUpSpied = false;
    const connection = ObsConnection({ onSetUpObs: () => { setUpSpied = true; } }) as AnyElement;
    const button = elementsOf(connection).find(element => element.type === 'button' && element.props.className === 'obs-setup');
    expect(button, 'Set up button').toBeTruthy();
    (button!.props as { onClick(): void }).onClick();
    expect(setUpSpied).toBe(true);
    const bare = ObsConnection({}) as AnyElement;
    expect(elementsOf(bare).some(element => element.type === 'button')).toBe(false);
    expect(renderToStaticMarkup(bare)).not.toContain('Set up in Settings');
  });
});

// The catalog view ObsNameField-based tests use: scenes plus one per-name list, and spies for
// refresh and on-demand fetches (the real view comes from useObsCatalog with the store).
function catalogView(overrides?: Partial<ObsCatalogView>): ObsCatalogView {
  return {
    desktop: true, status: 'ready', error: null, lists: {}, connection: null,
    names: { scenes: ['Games', 'Live'], sources: ['Cam', 'Mic'], scene_collections: ['Show'], profiles: ['Streaming'] },
    refresh: () => {}, list: () => null, requestNames: () => {},
    ...overrides,
  };
}

describe('obs name listings', () => {
  const renderAction = (action: BindingAction | null, catalog?: ObsCatalogView) =>
    renderToStaticMarkup(ActionFields({
      action, fader: false, selectorKey: 'm4315:root:0:pressed:', onChange: () => {}, onSetUpObs: () => {}, catalog,
    }));

  it('shows the connection chip in the editor only once a state is known', () => {
    const action: BindingAction = { type: 'obs_scene', scene: 'Live' };
    expect(renderAction(action, catalogView())).not.toContain('obs-chip');
    const down = renderAction(action, catalogView({ status: 'failed', error: 'could not connect', connection: { state: 'not_running', message: 'OBS is not running.' } }));
    expect(down).toContain('obs-chip-not_running" title="OBS is not running.">Not running</span>');
    expect(down).toContain('could not connect');
    expect(renderAction(action, catalogView({ connection: { state: 'connected', message: 'Connected to OBS.' } }))).toContain('>Connected</span>');
  });

  it('turns the OBS name fields into comboboxes only while the list is there', () => {
    const plain = renderAction({ type: 'obs_scene', scene: 'Live' });
    expect(plain).not.toContain('role="combobox"');
    expect(plain).not.toContain('Refresh from OBS');
    expectAccessible(plain);
    const withCatalog = renderAction({ type: 'obs_scene', scene: 'Live' }, catalogView());
    expect(withCatalog).toMatch(/id="obs-scene"[^>]*role="combobox"[^>]*aria-expanded="false"/);
    expect(withCatalog).toMatch(/aria-autocomplete="list"/);
    expect(withCatalog).toContain('Refresh from OBS');
    expectAccessible(withCatalog);
    // While listing, the fields stay comboboxes (their popups say so) and the press is off;
    // a failed listing leaves plain inputs, says why, and keeps the Refresh press.
    expect(renderAction({ type: 'obs_scene', scene: 'Live' }, catalogView({ status: 'loading', names: null }))).toContain('role="combobox"');
    const failed = renderAction({ type: 'obs_scene', scene: 'Live' }, catalogView({ status: 'failed', names: null, error: 'OBS at 127.0.0.1:4455: could not connect' }));
    expect(failed).not.toContain('role="combobox"');
    expect(failed).toContain('could not connect');
    expect(failed).not.toMatch(/disabled=""[^>]*>.*Refresh from OBS/);
    expectAccessible(failed);
  });

  it('warns when the exact saved name is not in the OBS list, and stays quiet when it is', () => {
    const missing = renderAction({ type: 'obs_scene', scene: 'Liev' }, catalogView());
    expect(missing).toContain('OBS does not list this scene right now. Refresh the OBS lists, or check the exact spelling.');
    expect(missing).toMatch(/aria-describedby="obs-scene-warning"/);
    expect(missing).not.toMatch(/aria-invalid="true"[^>]*aria-describedby="obs-scene-warning"/);
    expectAccessible(missing);
    const exact = renderAction({ type: 'obs_scene', scene: 'Live' }, catalogView());
    expect(exact).not.toContain('field-warning');
    // Other operations check their own lists, including scene collection and profile.
    expect(renderAction({ type: 'obs_profile', name: 'Ghost' }, catalogView()))
      .toContain('OBS does not list this profile right now.');
    expect(renderAction({ type: 'obs_scene_collection', name: 'Ghost' }, catalogView()))
      .toContain('OBS does not list this scene collection right now.');
    // A name rule error still wins over the live cross-check.
    const blank = renderAction({ type: 'obs_scene', scene: ' ' }, catalogView());
    expect(blank).not.toContain('field-warning');
    expect(blank).toContain('role="alert"');
  });

  it('offers a filter its source inputs plus scenes, and fetches filters per source on open', () => {
    const requested: [ObsNameKind, string][] = [];
    const view = catalogView({
      list: (kind, name) => kind === 'filters' && name === 'Cam' ? { status: 'ready', names: ['Blur'] } : null,
      requestNames: (kind, name) => requested.push([kind, name]),
    });
    const markup = renderAction({ type: 'obs_filter', source: 'Cam', filter: 'Blur', op: 'toggle' }, view);
    expect(markup).toMatch(/id="obs-filter-source"[^>]*role="combobox"/);
    expect(markup).toMatch(/id="obs-filter-name"[^>]*role="combobox"/);
    expect(markup).not.toContain('field-warning');
    // An unknown source says so; the filter of a known source stays quiet.
    const unknown = renderAction({ type: 'obs_filter', source: 'Ghost', filter: 'Blur', op: 'toggle' }, view);
    expect(unknown).toContain('OBS does not list this source right now.');
    expect(unknown).not.toContain('OBS does not list this filter');
    expectAccessible(unknown);
    // A visibility source is checked against the items of its scene, fetched on open.
    const inScene = catalogView({
      list: (kind, name) => kind === 'sceneSources' && name === 'Live' ? { status: 'ready', names: ['Cam'] } : null,
      requestNames: (kind, name) => requested.push([kind, name]),
    });
    const visibility = renderAction({ type: 'obs_source_visibility', scene: 'Live', source: 'Ghost', op: 'off' }, inScene);
    expect(visibility).toContain('OBS does not list this source right now.');
    const fine = renderAction({ type: 'obs_source_visibility', scene: 'Live', source: 'Cam', op: 'off' }, inScene);
    expect(fine).not.toContain('field-warning');
    expectAccessible(visibility);
  });

  it('shows one Refresh row per editor and refreshes on press', () => {
    let refreshed = false;
    const single = renderToStaticMarkup(ActionFields({
      action: { type: 'obs_scene', scene: 'Live' }, fader: false, selectorKey: 'k', onChange: () => {}, onSetUpObs: () => {},
      catalog: catalogView({ refresh: () => { refreshed = true; } }),
    }));
    expect((single.match(/Refresh from OBS/g) ?? []).length).toBe(1);
    expect(single).not.toMatch(/disabled=""[^>]*>.*Refresh from OBS/);
    const tree = ObsConnection({ onSetUpObs: () => {}, catalog: catalogView({ refresh: () => { refreshed = true; } }) }) as AnyElement;
    const refresh = elementsOf(tree).find(element => element.type === 'button' && String(element.props.className).includes('button compact'));
    expect(refresh, 'Refresh button').toBeTruthy();
    (refresh!.props as { onClick(): void }).onClick();
    expect(refreshed).toBe(true);
    // While listing, the press is off.
    const loading = ObsConnection({ catalog: catalogView({ status: 'loading', names: null }) }) as AnyElement;
    expect(renderToStaticMarkup(loading)).toMatch(/<button[^>]*disabled=""[^>]*>.*Listing…<\/button>/);
    // The browser preview (desktop false, nothing fetched) shows nothing new.
    const preview = ObsConnection({ catalog: { ...catalogView(), desktop: false, status: 'idle', names: null } }) as AnyElement;
    expect(renderToStaticMarkup(preview)).not.toContain('Refresh from OBS');
    const inStep = renderToStaticMarkup(ActionFields({
      action: { type: 'multi_action', actions: [{ type: 'obs_scene', scene: 'Live' }, { type: 'obs_transition' }] },
      fader: false, selectorKey: 'k', onChange: () => {}, onSetUpObs: () => {}, catalog: catalogView(),
    }));
    expect((inStep.match(/Refresh from OBS/g) ?? []).length).toBe(1);
    expectAccessible(inStep);
  });
});

describe('obs combo', () => {
  const ready: ObsNameList = { status: 'ready', names: ['Cam', 'cam 2', 'Games', 'Live', 'Mic', 'Title “2”'] };
  type ComboResult = { typed?: string; picked?: string; opened?: boolean; active?: number; requested?: boolean };
  const comboOf = (overrides: { value?: string; entry?: ObsNameList | null; open?: boolean; active?: number } = {}, onOpenList?: () => void) => {
    const result: ComboResult = {};
    const tree = ObsCombo({
      id: 'obs-scene', value: overrides.value ?? '', entry: 'entry' in overrides ? overrides.entry! : ready,
      open: overrides.open ?? false, onOpenChange: open => { result.opened = open; },
      active: overrides.active ?? 0, onActiveChange: at => { result.active = at; },
      onType: value => { result.typed = value; }, onPick: name => { result.picked = name; },
      onOpenList,
    }) as AnyElement;
    return { result, tree };
  };
  const inputOf = (tree: AnyElement) => {
    const input = elementsOf(tree).find(element => element.type === 'input');
    expect(input, 'combo input').toBeTruthy();
    return input!.props as {
      value: string; role?: string; 'aria-expanded'?: boolean; 'aria-controls'?: string; 'aria-activedescendant'?: string;
      onClick(event: { target: unknown }): void; onKeyDown(event: { key: string; preventDefault(): void }): void;
      onChange(event: { target: { value: string } }): void; onBlur(): void;
    };
  };
  const optionsOf = (tree: AnyElement) => elementsOf(tree).filter(element => element.props.role === 'option')
    .map(element => ({ element, props: element.props as { id: string; 'aria-selected'?: boolean; onClick(): void; onMouseDown(event: { preventDefault(): void }): void; className?: string } }));
  const notesOf = (tree: AnyElement) => elementsOf(tree).filter(element => element.props.className === 'combo-note')
    .map(element => renderToStaticMarkup(element as AnyElement));

  it('is a plain input without a list and a combobox with one', () => {
    const { tree } = comboOf({ entry: null });
    const input = inputOf(tree);
    expect(input.role).toBe(undefined);
    expect(input['aria-expanded']).toBe(undefined);
    expect(renderToStaticMarkup(tree)).toContain('value=""');
    const listed = inputOf(comboOf().tree);
    expect(listed.role).toBe('combobox');
    expect(listed['aria-expanded']).toBe(false);
    expect(listed['aria-controls']).toBe('obs-scene-choices');
  });

  it('opens on click, filtered by what is typed, and offers every savable name', () => {
    const { tree, result } = comboOf({ value: 'ca' });
    inputOf(tree).onClick({ target: null });
    expect(result.opened).toBe(true);
    expect(result.active).toBe(0);
    // Filtering is case-insensitive on the trimmed text.
    const open = comboOf({ value: 'ca', open: true });
    const names = optionsOf(open.tree).map(option => renderToStaticMarkup(option.element));
    expect(names).toHaveLength(2);
    expect(renderToStaticMarkup(open.tree)).not.toContain('Games');
    // Typing keeps the choices open and never closes them; picking does.
    const typing = comboOf({ open: true });
    inputOf(typing.tree).onChange({ target: { value: 'Cam' } });
    expect(typing.result.typed).toBe('Cam');
    expect(typing.result.opened).toBe(undefined);
    const pick = comboOf({ open: true, active: 1 });
    optionsOf(pick.tree)[1].props.onClick();
    expect(pick.result.picked).toBe('cam 2');
    expect(pick.result.opened).toBe(false);
    // The exact current value is marked selected, options keep the popup on mousedown.
    const current = comboOf({ value: 'Live', open: true });
    const selected = optionsOf(current.tree).find(option => option.props['aria-selected']);
    // The query "Live" leaves one match.
    expect(selected?.props.id).toBe('obs-scene-choice-0');
    selected!.props.onMouseDown({ preventDefault() {} });
  });

  it('moves with the arrow keys, picks with Enter and closes with Escape or blur', () => {
    const open = comboOf({ open: true, active: 0 });
    const input = inputOf(open.tree);
    input.onKeyDown({ key: 'ArrowDown', preventDefault() {} });
    expect(open.result.active).toBe(1);
    input.onKeyDown({ key: 'ArrowUp', preventDefault() {} });
    expect(open.result.active).toBe(0);
    // ArrowUp never leaves the first match.
    input.onKeyDown({ key: 'ArrowUp', preventDefault() {} });
    expect(open.result.active).toBe(0);
    // An out-of-range active option clamps to the last match, where ArrowDown stays.
    const atEnd = comboOf({ open: true, active: 99 });
    expect(renderToStaticMarkup(atEnd.tree)).toContain('aria-activedescendant="obs-scene-choice-5"');
    inputOf(atEnd.tree).onKeyDown({ key: 'ArrowDown', preventDefault() {} });
    expect(atEnd.result.active).toBe(5);
    const enter = comboOf({ open: true, active: 2 });
    inputOf(enter.tree).onKeyDown({ key: 'Enter', preventDefault() {} });
    expect(enter.result.picked).toBe('Games');
    expect(enter.result.opened).toBe(false);
    const closed = comboOf({ active: 2 });
    inputOf(closed.tree).onKeyDown({ key: 'Enter', preventDefault() {} });
    expect(closed.result.picked).toBe(undefined);
    for (const key of ['Escape', 'ArrowUp']) {
      const silent = comboOf({ active: 2 });
      inputOf(silent.tree).onKeyDown({ key, preventDefault() {} });
      expect(silent.result.opened, key).toBe(undefined);
    }
    const blur = comboOf({ open: true });
    inputOf(blur.tree).onBlur();
    expect(blur.result.opened).toBe(false);
    // ArrowDown opens a closed combobox.
    const opening = comboOf();
    inputOf(opening.tree).onKeyDown({ key: 'ArrowDown', preventDefault() {} });
    expect(opening.result.opened).toBe(true);
  });

  it('says what the list is doing and keeps long or unmatched names typing', () => {
    const loading = comboOf({ entry: { status: 'loading' }, open: true });
    expect(notesOf(loading.tree)).toHaveLength(1);
    expect(notesOf(loading.tree)[0]).toContain('Looking up OBS…');
    const failed = comboOf({ entry: { status: 'failed', error: 'OBS refused GetSourceFilterList (no such source)' }, open: true });
    expect(notesOf(failed.tree)[0]).toContain('OBS refused GetSourceFilterList (no such source)');
    const empty = comboOf({ value: 'zzz', open: true });
    expect(optionsOf(empty.tree)).toHaveLength(0);
    expect(notesOf(empty.tree)[0]).toContain('Nothing in the OBS list matches. The name you type is kept.');
    // Names a profile can never reference are not offered, but typing one is kept.
    const long: ObsNameList = { status: 'ready', names: ['x'.repeat(129), 'ok', 'y'.repeat(200)] };
    const filtered = comboOf({ entry: long, open: true });
    expect(optionsOf(filtered.tree).map(option => option.props.id)).toEqual(['obs-scene-choice-0']);
    inputOf(filtered.tree).onChange({ target: { value: 'x'.repeat(129) } });
    expect(filtered.result.typed).toBe('x'.repeat(129));
    // More matches than shown say how many; 50 stay offered.
    const many: ObsNameList = { status: 'ready', names: Array.from({ length: 55 }, (_, index) => `S${index}`) };
    const paged = comboOf({ entry: many, open: true });
    expect(optionsOf(paged.tree)).toHaveLength(50);
    expect(notesOf(paged.tree)[0]).toContain('5 more — keep typing to narrow the list.');
  });

  it('fetches per-name lists when its choices open', () => {
    let requested: string | null = null;
    const once = comboOf({}, () => { requested = 'once'; });
    inputOf(once.tree).onClick({ target: null });
    expect(requested).toBe('once');
    // Already open: no second fetch.
    const open = comboOf({ open: true }, () => { requested = 'twice'; });
    inputOf(open.tree).onClick({ target: null });
    expect(requested).toBe('once');
    // Without a list there is nothing to fetch.
    const plain = comboOf({ entry: null }, () => { requested = 'plain'; });
    inputOf(plain.tree).onClick({ target: null });
    expect(requested).toBe('once');
  });
});
