import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import preview from '../fixtures/dial-function-preview.json';
import type { Profile, ReplayEvent } from './contracts';
import { addDialFunction, editDialBinding, renameDialFunction } from './dial-function-model';
import { DialFunctionsPanel, bindingPreviewText, dialPreviewText, selectionAfterMove } from './DialFunctionsPanel';
import { ProfileSession } from './profile-session';
import { SessionView } from './ProfileManager';
import { dialSwitchLabel } from './ui-model';
import type { Invoke } from './profile-session';

const fixture = () => JSON.parse(readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/dial-functions.json', import.meta.url), 'utf8')) as Profile;
const plain: Profile = { schema_version: 1, name: 'P', bindings: [{ module: 'm131e', control: 0, gesture: 'short_press', action: { type: 'toggle_mute' } }] };

function harness(profile: Profile) {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const invoke: Invoke = <T,>(command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    return Promise.resolve((preview as Record<string, unknown>).rotation as T);
  };
  return { session: new ProfileSession(invoke, profile, false), calls };
}

const render = (profile: Profile, initialSelected = 0) => {
  const { session } = harness(profile);
  return renderToStaticMarkup(<DialFunctionsPanel profile={profile} session={session} state={session.getState()} disabled={false} initialSelected={initialSelected} />);
};

describe('DialFunctionsPanel markup', () => {
  it('hands the desktop backend to the action editor, so the front-app test and Open app offer the app picker', () => {
    const { session } = harness(plain);
    const invoke: Invoke = <T,>() => Promise.resolve(null as T);
    const withFlow = editDialBinding(fixture(), 0, 'right', '', { type: 'conditional', when: { focused_app: 'com.apple.finder' }, then: [{ type: 'launch_app', bundle_id: 'com.apple.Music' }] });
    const panel = (backend?: Invoke) => renderToStaticMarkup(<DialFunctionsPanel profile={withFlow} session={session} state={session.getState()} disabled={false} invoke={backend} />);
    const desktop = panel(invoke);
    expect(desktop).toMatch(/<select id="conditional-app"/);
    expect(desktop).toMatch(/<select id="then-step-1-app-bundle-id"/);
    expect(desktop.match(/Choose app…<\/button>/g)).toHaveLength(2);
    // The browser preview keeps the plain bundle ID fields.
    const browser = panel();
    expect(browser).toMatch(/<input id="conditional-app"[^>]*value="com.apple.finder"/);
    expect(browser).not.toContain('Choose app…');
  });

  it('offers to start dial functions without changing a profile that has none', () => {
    const html = render(plain);
    expect(html).toContain('aria-labelledby="dial-functions-heading"');
    expect(html).toContain('Add dial functions');
    expect(html).toContain('including Short press');
    expect(html).toContain('1 existing encoder binding must move');
    expect(html).not.toContain('dial-function-0-name');
  });

  it('labels every function, its move and delete buttons, and never offers Short press as a binding gesture', () => {
    const html = render(fixture());
    expect(html).toContain('3 of 16');
    for (const [index, name] of ['Volume', 'Scroll', 'Apps'].entries()) {
      expect(html).toContain(`<label for="dial-function-${index}-name">Function ${index + 1} name</label>`);
      expect(html).toContain(`value="${name}"`);
      expect(html).toContain(`aria-label="Delete ${name}"`);
    }
    expect(html).toContain('aria-label="Move Volume up"');
    expect(html).toMatch(/aria-label="Move Volume up"[^>]*disabled/);
    expect(html).toMatch(/aria-label="Move Apps down"[^>]*disabled/);
    const gestureSelect = html.slice(html.indexOf('id="dial-binding-gesture"'), html.indexOf('</select>', html.indexOf('id="dial-binding-gesture"')));
    expect(gestureSelect).toContain('value="right"');
    expect(gestureSelect).toContain('value="long_press"');
    expect(gestureSelect).not.toContain('short_press');
    expect(gestureSelect).not.toContain('pressed');
    // The existing action picker edits the selected binding.
    expect(html).toContain('id="binding-action"');
    expect(html).toContain('Previewing function: Volume');
    expect(html).toContain('aria-label="Remove Volume Right binding for serial desk-dial"');
    expect(html).not.toContain('Encoder bindings outside dial functions');
  });

  it('shows which function is edited and previewed', () => {
    const html = render(fixture(), 2);
    expect(html).toContain('Previewing function: Apps');
    expect(html).toContain('aria-label="Bindings of Apps"');
    expect(render(addDialFunction(fixture(), 'Off'))).toContain('No bindings: the encoder does nothing');
    expect(render(fixture(), 9)).toContain('Previewing function: Apps');
  });

  it('shows inline name errors mirroring the service and offers to move flat encoder bindings', () => {
    const profile = renameDialFunction(addDialFunction(plain, 'Main'), 0, ' Main');
    const html = render(profile);
    expect(html).toContain('aria-invalid="true"');
    expect(html).toContain('id="dial-function-0-name-error"');
    expect(html).toContain('start and end');
    expect(html).toContain('aria-label="Encoder bindings outside dial functions"');
    expect(html).toContain('1 binding will be dropped because no function can hold it: Short press.');
  });
});

describe('dial function previews', () => {
  it('describes rotation, unbound and switch results from the Rust fixture', () => {
    expect(dialPreviewText(preview.rotation as ReplayEvent, 'Scroll')).toBe('Scroll · Right: would j.');
    expect(dialPreviewText(preview.unbound as ReplayEvent, 'Scroll')).toBe('Scroll · Long press: no action is bound in this function.');
    expect(dialPreviewText(preview.switch as unknown as ReplayEvent, 'Apps')).toBe('Short press switches from Apps to Volume (1 of 3). No action runs.');
  });

  it('sends the dial function index only when one is chosen, keeping the existing call shape', async () => {
    const { session, calls } = harness(fixture());
    await session.previewBinding({ module: 'm131e', control: 0, gesture: 'right', serial: '', faderPosition: 0, dialFunction: 1 });
    await session.previewBinding({ module: 'm131e', control: 0, gesture: 'right', serial: '', faderPosition: 0 });
    const [chosen, plainCall] = calls.map(call => call.args!);
    const { profile: _profile, ...request } = chosen;
    expect(request).toEqual(preview.request);
    expect(plainCall).not.toHaveProperty('dialFunction');
  });

  it('dial function edits are undoable draft edits, and typing a name is one step', () => {
    const { session } = harness(plain);
    session.update(latest => addDialFunction(latest));
    for (const name of ['M', 'Ma', 'Main']) session.update(latest => renameDialFunction(latest, 0, name), 'dial-function-name:0');
    expect(session.getState().draft.profile.dial_functions).toEqual([{ name: 'Main', bindings: [] }]);
    expect(session.undo()).toBe(true);
    expect(session.getState().draft.profile.dial_functions).toEqual([{ name: 'Function 1', bindings: [] }]);
    expect(session.undo()).toBe(true);
    expect(session.getState().draft.profile).not.toHaveProperty('dial_functions');
    expect(session.redo()).toBe(true);
    expect(session.redo()).toBe(true);
    expect(session.getState().draft.profile.dial_functions![0].name).toBe('Main');
  });
});

describe('dial function replay rows', () => {
  const result = (events: unknown[]) => ({ status: 'completed' as const, name: 'r', result: { source: 's', synthetic: true as const, status: 'completed' as const, report_count: 1, event_count: events.length, planned_action_count: 1, events: events as ReplayEvent[], diagnostics: [] } });

  it('shows the function on rows and a distinct switch row that is not counted unbound', () => {
    const html = renderToStaticMarkup(<SessionView session={result([preview.rotation, preview.switch])} />);
    expect(html).toContain('Function: Scroll');
    expect(html).toContain('<span class="dial-switch">Dial function: Apps → Volume (1/3)</span>');
    expect(html).toContain('<strong>0</strong><span>No action bound</span>');
  });

  it('formats the switch label', () => {
    expect(dialSwitchLabel({ from_function: 'Volume', to_function: 'Brightness', index: 1, count: 3 })).toBe('Volume → Brightness (2/3)');
  });
});

describe('dial function review follow-ups', () => {
  it('selection follows the edited function when it or its neighbour moves', () => {
    expect(selectionAfterMove(1, 1, -1)).toBe(0);
    expect(selectionAfterMove(1, 0, 1)).toBe(0);
    expect(selectionAfterMove(1, 2, -1)).toBe(2);
    expect(selectionAfterMove(0, 2, -1)).toBe(0);
  });

  it('makes colliding and unmovable flat bindings explicit before moving', () => {
    const base = addDialFunction({ ...plain, bindings: [...plain.bindings,
      { module: 'm131e', control: 0, gesture: 'right', serial_number: 'unit', action: { type: 'volume', delta: 9 } },
      { module: 'm131e', control: 0, gesture: 'left', action: { type: 'volume', delta: -9 } },
    ] }, 'Main');
    const profile = editDialBinding(base, 0, 'right', 'unit', { type: 'toggle_mute' });
    const html = render(profile);
    expect(html).toContain('1 binding will be dropped because Main already has the same gesture: Right (serial unit).');
    expect(html).toContain('1 binding will be dropped because no function can hold it: Short press.');
    expect(html).toContain('Move 1 encoder binding into Main');
    expect(html).not.toContain('aria-pressed="true">Editing');
  });

  it('describes encoder previews with the function, and keeps the plain wording otherwise', () => {
    expect(bindingPreviewText(preview.first_function as ReplayEvent)).toBe('Volume · Right: would output volume +5 points.');
    expect(bindingPreviewText(preview.switch as unknown as ReplayEvent)).toContain('switches from Apps to Volume');
    expect(bindingPreviewText({ timestamp_us: 0, module: 'm131e', control: 0, gesture: 'short_press', serial_number: null, action: { type: 'toggle_mute' } })).toBe('Short press: Would toggle output mute.');
  });
});
