import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import preview from '../fixtures/dial-function-preview.json';
import type { Profile, ReplayEvent } from './contracts';
import { addDialFunction, editDialBinding, renameDialFunction } from './dial-function-model';
import { DialFunctionsPanel, bindingPreviewText, dialActionCount, dialGestureLabel, dialGestureRows, dialPreviewText, flatBindingOutcomes, selectionAfterMove } from './DialFunctionsPanel';
import { flatDialBindings } from './dial-function-model';
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
    expect(html).toContain('No functions yet; the 1 existing encoder binding will need to move into one.');
    expect(html).not.toContain('dial-function-0-name');
  });

  it('keeps the explanation and naming rules behind one disclosure, with one short line visible', () => {
    const html = render(fixture());
    expect(html).toContain('<p class="dial-functions-intro">Short press switches function; turning and long press run the current one.</p>');
    const details = html.slice(html.indexOf('<details class="learn-more"><summary>How functions work</summary>'), html.indexOf('</details>'));
    expect(details).toContain('never runs an action');
    expect(details).toContain('Names must be unique, at most 64 bytes');
    // The rules are not repeated as permanent fine print.
    expect(html.split('Names must be unique')).toHaveLength(2);
  });

  it('shows each function as a compact row with a hidden name label, a summary and labelled icon buttons', () => {
    const html = render(fixture());
    expect(html).toContain('3 of 16');
    for (const [index, name, count] of [[0, 'Volume', '4 actions'], [1, 'Scroll', '2 actions'], [2, 'Apps', '1 action']] as const) {
      expect(html).toContain(`<label class="sr-only" for="dial-function-${index}-name">Function ${index + 1} name</label>`);
      expect(html).toContain(`value="${name}"`);
      expect(html).toContain(`aria-label="Edit ${name}, ${count}">${count}</button>`);
      expect(html).toContain(`aria-label="Delete ${name}"`);
    }
    expect(html).toMatch(/aria-pressed="true" aria-label="Edit Volume, 4 actions"/);
    expect(html).toMatch(/aria-pressed="false" aria-label="Edit Scroll, 2 actions"/);
    expect(html).toMatch(/aria-label="Move Volume up"[^>]*disabled/);
    expect(html).toMatch(/aria-label="Move Apps down"[^>]*disabled/);
    expect(html).not.toContain('Edit bindings');
    expect(render(addDialFunction(fixture(), 'Off'))).toContain('aria-label="Edit Off, No actions"');
  });

  it('lists one row per gesture with its action or Not set, and never offers Short press as a binding', () => {
    const html = render(fixture(), 2);
    expect(html).toContain('aria-label="Bindings of Apps"');
    expect(html).not.toContain('id="dial-binding-gesture"');
    expect(html).toMatch(/aria-pressed="true"><strong>Turn right<\/strong><span class="muted">Not set<\/span>/);
    expect(html).toMatch(/aria-pressed="false"><strong>Turn left<\/strong><span class="muted">Not set<\/span>/);
    expect(html).toMatch(/<strong>Long press<\/strong><span>Open com.apple.finder/);
    expect(html).toContain('aria-label="Clear Long press"');
    expect(html).not.toContain('aria-label="Clear Turn right"');
    expect(html).toContain('<strong>Short press</strong><span class="muted">Switches function; cannot be bound</span>');
    expect(html).not.toMatch(/<button[^>]*aria-pressed[^>]*><strong>Short press/);
    // Only the selected gesture's action picker is shown, under its row.
    expect(html.match(/id="binding-action"/g)).toHaveLength(1);
    expect(html.indexOf('id="binding-action"')).toBeGreaterThan(html.indexOf('Turn right'));
    expect(html.indexOf('id="binding-action"')).toBeLessThan(html.indexOf('Turn left'));
    // The selected function's editor sits inside its row; an out-of-range selection falls back to the last one.
    expect(render(fixture(), 9)).toContain('aria-label="Bindings of Apps"');
  });

  it('puts a preview button on every gesture row, Short press included', () => {
    const html = render(fixture());
    for (const label of ['Turn right', 'Turn left', 'Long press', 'Short press']) expect(html).toContain(`aria-label="Preview ${label} in Volume"`);
    expect(html).not.toContain('Previewing function');
  });

  it('keeps the device serial under a collapsed Advanced toggle and notes serial-specific bindings', () => {
    const html = render(fixture());
    expect(html).toContain('aria-expanded="false" aria-controls="dial-function-advanced"');
    expect(html).not.toContain('id="dial-binding-serial"');
    expect(html).toContain('<strong>Turn right</strong><span>Output volume +5 points · +1 for other serials</span>');
  });

  it('shows inline name errors mirroring the service and offers to move flat encoder bindings', () => {
    const profile = renameDialFunction(addDialFunction(plain, 'Main'), 0, ' Main');
    const html = render(profile);
    expect(html).toContain('aria-invalid="true"');
    expect(html).toContain('id="dial-function-0-name-error"');
    expect(html).toContain('start and end');
    expect(html).toContain('aria-label="Encoder bindings outside dial functions"');
    expect(html).toContain('1 encoder binding isn&#x27;t in a function yet and the profile can&#x27;t be saved until it is.');
    expect(html).toContain('<summary>What happens to each binding</summary>');
    expect(html).toContain('1 binding will be dropped because no function can hold it: Short press.');
    expect(html).toContain('Remove the encoder bindings outside functions');
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
    expect(html).toContain('1 binding will move into Main: Left.');
    expect(html).toContain('Move 1 encoder binding into Main');
    // One primary action; the details stay in the disclosure, with the undo note.
    const banner = html.slice(html.indexOf('class="dial-flat-banner"'), html.indexOf('<ol class="dial-function-list">'));
    expect(banner.match(/<button/g)).toHaveLength(1);
    expect(banner).toContain('<li>Undo restores them.</li>');
  });

  it('summarises gesture rows, function rows and flat binding outcomes', () => {
    const volume = fixture().dial_functions![0];
    expect(dialGestureRows(volume, '')).toEqual([
      { gesture: 'right', label: 'Turn right', summary: 'Output volume +5 points', otherSerials: 1 },
      { gesture: 'left', label: 'Turn left', summary: 'Output volume -5 points', otherSerials: 0 },
      { gesture: 'long_press', label: 'Long press', summary: 'Toggle output mute', otherSerials: 0 },
    ]);
    expect(dialGestureRows(volume, 'desk-dial').map(row => [row.summary, row.otherSerials])).toEqual([['Output volume +1 points', 1], [null, 1], [null, 1]]);
    expect(dialGestureLabel('left')).toBe('Turn left');
    expect(dialActionCount(volume)).toBe('4 actions');
    expect(dialActionCount({ name: 'X', bindings: [] })).toBe('No actions');
    expect(flatBindingOutcomes(flatDialBindings(plain), 'Main')).toEqual(['1 binding will be dropped because no function can hold it: Short press.']);
  });

  it('describes encoder previews with the function, and keeps the plain wording otherwise', () => {
    expect(bindingPreviewText(preview.first_function as ReplayEvent)).toBe('Volume · Right: would output volume +5 points.');
    expect(bindingPreviewText(preview.switch as unknown as ReplayEvent)).toContain('switches from Apps to Volume');
    expect(bindingPreviewText({ timestamp_us: 0, module: 'm131e', control: 0, gesture: 'short_press', serial_number: null, action: { type: 'toggle_mute' } })).toBe('Short press: Would toggle output mute.');
  });
});
