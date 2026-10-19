import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Action } from './contracts';
import {
  defaultMacro, defaultMultiAction, defaultProfileAction, defaultStepAction, defaultTimerAction, isFaderTemplate, keyLabel, macroDurationLabel, macroDurationMs, macroIssue,
  macroLimits, maxObsSceneBytes, maxTimerLabelBytes,
  namedKeys, obsNameIssue, obsOperationLabels, obsOutputOpLabels, obsReplayBufferOpLabels, obsRecordOpLabels, obsSceneIssue, obsSwitchOpLabels,
  profileDirectionLabels, stepActionTypes, switchObsOperation, textIssue, timerDuration, timerLabelIssue, timerSeconds,
  timerSoundLabels, urlIssue,
} from './action-model';
import { actionLabel } from './ui-model';

// Shared with the Rust service tests (profile.rs), so both sides agree on action shapes.
const fixture = JSON.parse(readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/actions.json', import.meta.url), 'utf8')) as {
  named_keys: string[]; timer_sounds: string[]; valid: Action[]; invalid: unknown[];
};

describe('timer contract', () => {
  type LooseTimer = { type?: string; seconds?: unknown; label?: unknown; sound?: unknown; actions?: { type?: string }[] };
  // Mirrors the Rust checks for one timer action, so the fixture's invalid entries are explained on this side too.
  const timerAccepted = (action: LooseTimer) =>
    Object.keys(action).every(key => ['type', 'seconds', 'label', 'sound'].includes(key))
    && Number.isInteger(action.seconds) && (action.seconds as number) >= timerSeconds.minimum && (action.seconds as number) <= timerSeconds.maximum
    && (action.label === undefined || (typeof action.label === 'string' && action.label !== '' && timerLabelIssue(action.label) === null))
    && (action.sound === undefined || Object.keys(timerSoundLabels).includes(String(action.sound)));

  it('mirrors the closed sound set in order', () => {
    expect(Object.keys(timerSoundLabels)).toEqual(fixture.timer_sounds);
  });

  it('agrees with the shared fixture on valid and invalid timers', () => {
    const valid = fixture.valid.filter(action => action.type === 'timer');
    expect(valid.length).toBeGreaterThan(0);
    for (const action of valid) expect(timerAccepted(action), JSON.stringify(action)).toBe(true);
    const invalid = (fixture.invalid as LooseTimer[]).filter(action => action.type === 'timer');
    expect(invalid.length).toBeGreaterThan(5);
    for (const action of invalid) expect(timerAccepted(action), JSON.stringify(action)).toBe(false);
    // Never a multi-action step, on either side.
    expect(stepActionTypes.map(([type]) => type)).not.toContain('timer');
    expect((fixture.invalid as LooseTimer[]).some(action => action.type === 'multi_action' && action.actions?.some(step => step.type === 'timer'))).toBe(true);
  });

  it('flags labels the service rejects', () => {
    for (const label of ['', 'Tea', 'Stream break — 5 min', '"; do shell script "id" `id` $(id)', 'é'.repeat(maxTimerLabelBytes / 2)]) {
      expect(timerLabelIssue(label), label).toBeNull();
    }
    for (const label of [' ', ' lead', 'trail ', 'line\nbreak', 'tab\tx', 'rtl‮txt', 'zw​sp', 'x'.repeat(maxTimerLabelBytes + 1)]) {
      expect(timerLabelIssue(label), JSON.stringify(label)).not.toBeNull();
    }
  });

  it('labels timers with their duration and builds a valid default', () => {
    expect(timerDuration(1)).toBe('0:01');
    expect(timerDuration(300)).toBe('5:00');
    expect(timerDuration(3661)).toBe('1:01:01');
    expect(timerDuration(86_400)).toBe('24:00:00');
    expect(actionLabel({ type: 'timer', seconds: 90, label: 'Tea' })).toBe('Timer 1:30 “Tea”');
    expect(actionLabel(defaultTimerAction())).toBe('Timer 5:00');
    expect(timerAccepted(defaultTimerAction() as LooseTimer)).toBe(true);
  });
});

describe('macro contract', () => {
  type LooseMacro = { type?: string; steps?: unknown };
  type LooseStep = { key?: unknown; modifiers?: unknown; delay_before_ms?: unknown; hold_ms?: unknown };
  // Mirrors the Rust checks for one macro action, so the fixture's entries are explained on this side too.
  const macroAccepted = (action: LooseMacro) => {
    if (action.type !== 'macro' || !Array.isArray(action.steps)) return false;
    const steps = action.steps as LooseStep[];
    if (steps.length < macroLimits.steps.minimum || steps.length > macroLimits.steps.maximum) return false;
    let total = 0;
    for (const step of steps) {
      if (Object.keys(step).some(key => !['key', 'modifiers', 'delay_before_ms', 'hold_ms'].includes(key))) return false;
      const named = namedKeys.some(([name]) => name === step.key);
      const literal = typeof step.key === 'string' && step.key.length === 1 && /[a-z0-9 ]/i.test(step.key);
      if (!literal && !named) return false;
      if (!Array.isArray(step.modifiers) || step.modifiers.length > 4 || new Set(step.modifiers.map(String)).size !== step.modifiers.length) return false;
      if (!Number.isInteger(step.delay_before_ms) || (step.delay_before_ms as number) > macroLimits.delayMs.maximum) return false;
      if (!Number.isInteger(step.hold_ms) || (step.hold_ms as number) > macroLimits.holdMs.maximum) return false;
      total += (step.delay_before_ms as number) + (step.hold_ms as number);
      if (total > macroLimits.durationMs) return false;
    }
    return true;
  };

  it('agrees with the shared fixture on valid and invalid macros', () => {
    const valid = fixture.valid.filter(action => action.type === 'macro');
    expect(valid.length).toBeGreaterThan(0);
    for (const action of valid) expect(macroAccepted(action), JSON.stringify(action)).toBe(true);
    const invalid = (fixture.invalid as LooseMacro[]).filter(action => action.type === 'macro');
    expect(invalid.length).toBeGreaterThan(3);
    for (const action of invalid) expect(macroAccepted(action), JSON.stringify(action)).toBe(false);
    // Never a multi-action step, on either side.
    expect(stepActionTypes.map(([type]) => type)).not.toContain('macro');
  });

  it('names every key the Mac can send, including the keypad and the media keys', () => {
    expect(keyLabel('keypad_enter')).toBe('Keypad Enter');
    expect(keyLabel('keypad_0')).toBe('Keypad 0');
    expect(keyLabel('play_pause')).toBe('Play/Pause media key');
    expect(keyLabel(' ')).toBe('Space');
    expect(keyLabel('c')).toBe('C');
  });

  it('says why a macro is refused and what it plans to spend', () => {
    expect(macroIssue([])).toContain('1 to 64 keystrokes');
    expect(macroIssue([{ key: 'a', modifiers: [], delay_before_ms: 0, hold_ms: 0 }])).toBeNull();
    expect(macroIssue([{ key: 'play_pause', modifiers: [], delay_before_ms: 0, hold_ms: 0 }])).toContain('media keys cannot be macro keys');
    expect(macroIssue([{ key: '"', modifiers: [], delay_before_ms: 0, hold_ms: 0 }])).toContain('pick a key');
    expect(macroIssue([{ key: 'a', modifiers: ['command', 'command'], delay_before_ms: 0, hold_ms: 0 }])).toContain('each modifier once');
    expect(macroIssue([{ key: 'a', modifiers: [], delay_before_ms: 10001, hold_ms: 0 }])).toContain('wait must be 0 to 10,000 ms');
    expect(macroIssue([{ key: 'a', modifiers: [], delay_before_ms: 0, hold_ms: 5001 }])).toContain('hold must be 0 to 5,000 ms');
    const steps = Array.from({ length: 3 }, () => ({ key: 'a', modifiers: [], delay_before_ms: 10000, hold_ms: 5000 }));
    expect(macroIssue(steps)).toContain('total at most 0:30');
    expect(macroDurationLabel(80)).toBe('0:00.1');
    expect(macroDurationLabel(30_000)).toBe('0:30');
    expect(macroDurationMs(steps)).toBe(45_000);
    expect(macroAccepted(defaultMacro())).toBe(true);
  });
});

describe('action contract', () => {
  it('mirrors the Rust named-key allow-list in order', () => {
    expect(namedKeys.map(([name]) => name)).toEqual(fixture.named_keys);
  });

  it('labels every valid fixture action without throwing', () => {
    for (const action of fixture.valid) expect(actionLabel(action)).not.toBe('');
    expect(actionLabel({ type: 'shortcut', key: 'arrow_left', modifiers: ['command', 'shift'] })).toBe('Command + Shift + Left Arrow');
    expect(actionLabel({ type: 'media', command: 'next_track' })).toBe('Next track');
    expect(actionLabel({ type: 'system', command: 'screenshot' })).toBe('Screenshot to clipboard');
    expect(actionLabel({ type: 'open_url', url: 'https://example.com' })).toBe('Open https://example.com');
    expect(actionLabel({ type: 'text', text: 'Hello\nthere, this is a long snippet', press_enter: true })).toBe('Paste “Hello there, this is a l…” + Return');
    expect(actionLabel(defaultMultiAction())).toBe('2 actions: Toggle output mute, then Command + C');
    expect(actionLabel({ type: 'profile', direction: 'previous' })).toBe('Previous profile');
    expect(actionLabel(defaultProfileAction())).toBe('Next profile');
  });

  it('mirrors the closed set of profile directions in the fixture', () => {
    const directions = fixture.valid.flatMap(action => action.type === 'profile' ? [action.direction] : []);
    expect(directions).toEqual(Object.keys(profileDirectionLabels));
    // Profile keys are never multi-action steps.
    expect(stepActionTypes.map(([type]) => type)).not.toContain('profile');
    const invalid = fixture.invalid as { type?: string; direction?: unknown; actions?: { type?: string }[] }[];
    expect(invalid.some(action => action.type === 'multi_action' && action.actions?.some(step => step.type === 'profile'))).toBe(true);
    // Each invalid profile action has a direction outside the set or a field the service refuses (no select by name).
    for (const action of invalid.filter(action => action.type === 'profile')) {
      const strict = Object.keys(action).every(key => key === 'type' || key === 'direction');
      expect(strict && Object.keys(profileDirectionLabels).includes(String(action.direction)), JSON.stringify(action)).toBe(false);
    }
  });

  it('builds default step actions that are themselves valid fixture shapes', () => {
    for (const [type] of stepActionTypes) {
      const action = defaultStepAction(type);
      expect(action.type).toBe(type);
      if (action.type === 'open_url') expect(urlIssue(action.url)).toBeNull();
      if (action.type === 'text') expect(textIssue(action.text)).toBeNull();
    }
    expect(stepActionTypes.map(([type]) => type)).not.toContain('multi_action');
  });

  it('labels named and literal keys', () => {
    expect(keyLabel('f20')).toBe('F20');
    expect(keyLabel('page_up')).toBe('Page Up');
    expect(keyLabel(' ')).toBe('Space');
    expect(keyLabel('q')).toBe('Q');
  });
});

describe('inline action hints', () => {
  it('flags URLs the service rejects', () => {
    for (const url of ['https://example.com', 'http://localhost:8080/x?y=1#z']) expect(urlIssue(url)).toBeNull();
    for (const url of ['', 'example.com', 'ftp://example.com', 'javascript:alert(1)', 'https://', 'https://user:pw@example.com',
      'https://example.com/"; do shell script "id"', 'https://exa mple.com', `https://example.com/${'a'.repeat(2048)}`]) {
      expect(urlIssue(url), url).not.toBeNull();
    }
  });

  it('flags text the service rejects', () => {
    expect(textIssue('line\n\ttab')).toBeNull();
    expect(textIssue('"; do shell script "id"')).toBeNull();
    for (const text of ['', 'bell\u0007', 'cr\r', 'esc\u001b[0m', 'x'.repeat(2049), 'é'.repeat(1025)]) expect(textIssue(text), JSON.stringify(text)).not.toBeNull();
  });

  it('rejects invisible format and separator characters like the Rust validator', () => {
    const invisible: number[] = [];
    for (let code = 0x200b; code <= 0x200f; code++) invisible.push(code);
    for (let code = 0x202a; code <= 0x202e; code++) invisible.push(code);
    for (let code = 0x2060; code <= 0x2069; code++) invisible.push(code);
    invisible.push(0x2028, 0x2029, 0xfeff, 0x00ad, 0x061c, 0x180e, 0x206f, 0xfff9, 0xe0001, 0xe0041);
    for (const code of invisible) expect(textIssue(`safe${String.fromCodePoint(code)}text`), code.toString(16)).toMatch(/invisible/);
    for (const text of ['Zażółć gęślą jaźń ĄĆĘŁŃÓŚŹŻ', 'Thanks! 🎉👍', 'heart ❤\u{FE0F} (with variation selector)', 'flag 🇵🇱', '日本語 ελληνικά русский', 'tab\tand\nnewline']) {
      expect(textIssue(text), text).toBeNull();
    }
    expect(textIssue('family 👨\u{200D}👩\u{200D}👧')).not.toBeNull();
  });

  it('agrees with the shared fixture on text snippets', () => {
    for (const action of fixture.valid) if (action.type === 'text') expect(textIssue(action.text), action.text).toBeNull();
    const invalid = fixture.invalid.filter((action): action is { type: 'text'; text: string } => (action as { type?: string }).type === 'text');
    expect(invalid.length).toBeGreaterThan(0);
    for (const action of invalid) expect(textIssue(action.text), JSON.stringify(action.text)).not.toBeNull();
  });
});

describe('obs source actions', () => {
  it('offers studio mode, the transition, filters and source visibility with safe defaults', () => {
    expect(stepActionTypes.map(([type]) => type)).toEqual(expect.arrayContaining(['obs_studio_mode', 'obs_transition', 'obs_filter', 'obs_source_visibility']));
    expect(defaultStepAction('obs_studio_mode')).toEqual({ type: 'obs_studio_mode', op: 'toggle' });
    expect(defaultStepAction('obs_transition')).toEqual({ type: 'obs_transition' });
    const filter = defaultStepAction('obs_filter') as { source: string; filter: string };
    const visibility = defaultStepAction('obs_source_visibility') as { scene: string; source: string };
    for (const [name, what] of [[filter.source, 'source'], [filter.filter, 'filter'], [visibility.scene, 'scene'], [visibility.source, 'source']] as const) expect(obsNameIssue(name, what)).toBeNull();
    expect(Object.keys(obsSwitchOpLabels)).toEqual(['toggle', 'on', 'off']);
    expect(actionLabel({ type: 'obs_studio_mode', op: 'on' })).toBe('OBS studio mode: Turn on');
    expect(actionLabel({ type: 'obs_transition' })).toBe('OBS studio transition');
    expect(actionLabel({ type: 'obs_filter', source: 'Cam', filter: 'Blur', op: 'off' })).toBe('OBS filter “Blur”: Turn off');
    expect(actionLabel({ type: 'obs_source_visibility', scene: 'Live', source: 'x'.repeat(40), op: 'toggle' })).toBe(`OBS source “${'x'.repeat(24)}…”: Toggle`);
    // Clipped by code points: an emoji at the cut is kept whole.
    expect(actionLabel({ type: 'obs_filter', source: 'Cam', filter: `${'x'.repeat(23)}🎥tail`, op: 'on' })).toBe(`OBS filter “${'x'.repeat(23)}🎥…”: Turn on`);
  });

  it('applies the scene-name rules to every name and says which kind it wants', () => {
    for (const what of ['scene', 'source', 'filter'] as const) {
      expect(obsNameIssue('Camera', what)).toBeNull();
      expect(obsNameIssue('x'.repeat(maxObsSceneBytes), what)).toBeNull();
      expect(obsNameIssue('', what)).toContain(`OBS ${what} name`);
      for (const bad of ['   ', 'x'.repeat(maxObsSceneBytes + 1), 'line\nbreak', 'zero\u200Bwidth', 'bidi\u202E']) expect(obsNameIssue(bad, what), JSON.stringify(bad)).not.toBeNull();
    }
    expect(obsSceneIssue('')).toBe(obsNameIssue('', 'scene'));
  });

  it('agrees with the shared fixture on which source actions are valid', () => {
    const names = (action: Record<string, unknown>) => (['scene', 'source', 'filter'] as const).filter(key => key in action);
    const strictlyValid = (action: Record<string, unknown>) => {
      const type = String(action.type);
      const allowed: Record<string, string[]> = { obs_studio_mode: ['type', 'op'], obs_transition: ['type'], obs_filter: ['type', 'source', 'filter', 'op'], obs_source_visibility: ['type', 'scene', 'source', 'op'] };
      if (!(type in allowed)) return null;
      const required = allowed[type];
      if (Object.keys(action).some(key => !required.includes(key)) || required.some(key => !(key in action))) return false;
      if ('op' in action && !['toggle', 'on', 'off'].includes(String(action.op))) return false;
      return names(action).every(key => typeof action[key] === 'string' && obsNameIssue(action[key] as string, key) === null);
    };
    for (const action of fixture.valid as Record<string, unknown>[]) if (strictlyValid(action) !== null) expect(strictlyValid(action), JSON.stringify(action)).toBe(true);
    const invalid = (fixture.invalid as Record<string, unknown>[]).filter(action => strictlyValid(action) !== null);
    expect(invalid.length).toBeGreaterThan(8);
    for (const action of invalid) expect(strictlyValid(action), JSON.stringify(action)).toBe(false);
  });
});

describe('obs scene collection and profile', () => {
  it('offers both as steps with valid defaults and clipped labels', () => {
    expect(stepActionTypes.map(([type]) => type)).toEqual(expect.arrayContaining(['obs_scene_collection', 'obs_profile']));
    const collection = defaultStepAction('obs_scene_collection') as { name: string };
    const profile = defaultStepAction('obs_profile') as { name: string };
    expect(obsNameIssue(collection.name, 'scene collection')).toBeNull();
    expect(obsNameIssue(profile.name, 'profile')).toBeNull();
    expect(obsNameIssue('', 'profile')).toContain('OBS profile name');
    expect(obsNameIssue('', 'scene collection')).toContain('OBS scene collection name');
    expect(actionLabel({ type: 'obs_scene_collection', name: 'Show' })).toBe('OBS scene collection “Show”');
    expect(actionLabel({ type: 'obs_profile', name: 'x'.repeat(40) })).toBe(`OBS profile “${'x'.repeat(24)}…”`);
  });

  it('agrees with the shared fixture on which of them are valid', () => {
    const strict = (action: Record<string, unknown>) => Object.keys(action).sort().join() === 'name,type' && typeof action.name === 'string' && obsNameIssue(action.name, 'profile') === null;
    const mine = (list: unknown) => (list as Record<string, unknown>[]).filter(action => action.type === 'obs_scene_collection' || action.type === 'obs_profile');
    expect(mine(fixture.valid).length).toBe(2);
    for (const action of mine(fixture.valid)) expect(strict(action), JSON.stringify(action)).toBe(true);
    expect(mine(fixture.invalid).length).toBeGreaterThan(3);
    for (const action of mine(fixture.invalid)) expect(strict(action), JSON.stringify(action)).toBe(false);
  });
});

describe('obs outputs', () => {
  it('offers the virtual camera and the replay buffer with closed operation sets, as steps and top-level actions', () => {
    expect(stepActionTypes.map(([type]) => type)).toEqual(expect.arrayContaining(['obs_virtual_camera', 'obs_replay_buffer']));
    expect(defaultStepAction('obs_virtual_camera')).toEqual({ type: 'obs_virtual_camera', op: 'toggle' });
    expect(defaultStepAction('obs_replay_buffer')).toEqual({ type: 'obs_replay_buffer', op: 'save' });
    expect(Object.keys(obsOutputOpLabels)).toEqual(['toggle', 'start', 'stop']);
    expect(Object.keys(obsReplayBufferOpLabels)).toEqual(['toggle', 'start', 'stop', 'save']);
    expect(actionLabel({ type: 'obs_virtual_camera', op: 'stop' })).toBe('OBS virtual camera: Stop');
    expect(actionLabel({ type: 'obs_replay_buffer', op: 'save' })).toBe('OBS replay buffer: Save the last seconds');
  });

  it('keeps the fixture and the labels in step', () => {
    const ops = (type: string) => fixture.valid.filter(action => action.type === type).flatMap(action => ('op' in action ? [String(action.op)] : []));
    expect(ops('obs_virtual_camera').sort()).toEqual(Object.keys(obsOutputOpLabels).sort());
    expect(ops('obs_replay_buffer').sort()).toEqual(Object.keys(obsReplayBufferOpLabels).sort());
    expect([...new Set(ops('obs_stream'))].sort()).toEqual(Object.keys(obsOutputOpLabels).sort());
    expect(ops('obs_record').sort()).toEqual(Object.keys(obsRecordOpLabels).sort());
  });

  it('offers mute, volume step and media source steps with their defaults and labels', () => {
    expect(defaultStepAction('obs_input_mute')).toMatchObject({ type: 'obs_input_mute', op: 'toggle' });
    expect(defaultStepAction('obs_adjust_input_volume')).toMatchObject({ type: 'obs_adjust_input_volume', step_db: 3 });
    expect(defaultStepAction('obs_media_source')).toMatchObject({ type: 'obs_media_source', op: 'play_pause' });
    expect(actionLabel({ type: 'obs_input_mute', input: 'Mic', op: 'mute' })).toBe('OBS input “Mic”: Mute');
    expect(actionLabel({ type: 'obs_adjust_input_volume', input: 'Mic', step_db: -3 })).toBe('OBS input “Mic” -3 dB');
    expect(actionLabel({ type: 'obs_media_source', input: 'Intro', op: 'play_stop' })).toBe('OBS media “Intro”: Play / stop');
  });

  it('offers the stream and the recording as steps with their own operation sets', () => {
    expect(defaultStepAction('obs_stream')).toEqual({ type: 'obs_stream', op: 'toggle' });
    expect(defaultStepAction('obs_record')).toEqual({ type: 'obs_record', op: 'toggle' });
    expect(actionLabel({ type: 'obs_stream', op: 'start' })).toBe('OBS stream: Start');
    expect(actionLabel({ type: 'obs_record', op: 'toggle_pause' })).toBe('OBS recording: Pause or resume');
  });
});

describe('obs_scene', () => {
  it('mirrors the Rust scene-name rules, including the shared fixture', () => {
    for (const scene of ['Live', '  padded  ', 'Zażółć 🎥 "quoted" {json}', 'x'.repeat(maxObsSceneBytes)]) expect(obsSceneIssue(scene), scene).toBeNull();
    for (const scene of ['', '   ', 'x'.repeat(maxObsSceneBytes + 1), 'é'.repeat(maxObsSceneBytes / 2 + 1), 'line\nbreak', 'tab\t', 'nul\u0000', 'esc\u001b[0m', 'zero\u200Bwidth', 'bidi\u202E', 'sep\u2028', 'bom\uFEFF']) {
      expect(obsSceneIssue(scene), JSON.stringify(scene)).not.toBeNull();
    }
    for (const action of fixture.valid) if (action.type === 'obs_scene') expect(obsSceneIssue(action.scene), action.scene).toBeNull();
    const invalid = (fixture.invalid as { type?: string; scene?: unknown }[]).filter(action => action.type === 'obs_scene');
    expect(invalid.length).toBeGreaterThan(0);
    for (const action of invalid) {
      // Either the name fails the shared rule, or the shape carries a field the service refuses.
      const strict = Object.keys(action).every(key => key === 'type' || key === 'scene') && typeof action.scene === 'string';
      expect(strict && obsSceneIssue(action.scene as string) === null, JSON.stringify(action)).toBe(false);
    }
  });

  it('is a multi-action step type with a valid default and a label', () => {
    expect(stepActionTypes.map(([type]) => type)).toContain('obs_scene');
    const action = defaultStepAction('obs_scene');
    expect(action).toEqual({ type: 'obs_scene', scene: 'Scene' });
    expect(obsSceneIssue((action as { scene: string }).scene)).toBeNull();
    expect(actionLabel({ type: 'obs_scene', scene: 'Starting soon' })).toBe('OBS scene “Starting soon”');
    expect(actionLabel({ type: 'obs_scene', scene: 'x'.repeat(40) })).toBe(`OBS scene “${'x'.repeat(24)}…”`);
  });
});

describe('obs operations', () => {
  it('names the nine obs step actions as editor operations', () => {
    const obsTypes = stepActionTypes.map(([type]) => type).filter(type => type.startsWith('obs_'));
    expect(Object.keys(obsOperationLabels)).toEqual(obsTypes);
  });

  it('switches operation, keeping the names both operations share', () => {
    expect(switchObsOperation({ type: 'obs_scene', scene: 'Live' }, 'obs_source_visibility'))
      .toEqual({ type: 'obs_source_visibility', scene: 'Live', source: 'Source', op: 'toggle' });
    // Operation settings reset; names the new operation does not have are dropped.
    expect(switchObsOperation({ type: 'obs_filter', source: 'Cam', filter: 'Blur', op: 'off' }, 'obs_source_visibility'))
      .toEqual({ type: 'obs_source_visibility', scene: 'Scene', source: 'Cam', op: 'toggle' });
    expect(switchObsOperation({ type: 'obs_scene', scene: 'Live' }, 'obs_replay_buffer'))
      .toEqual({ type: 'obs_replay_buffer', op: 'save' });
    expect(switchObsOperation({ type: 'obs_profile', name: 'Streaming' }, 'obs_scene_collection'))
      .toEqual({ type: 'obs_scene_collection', name: 'Streaming' });
    expect(switchObsOperation({ type: 'obs_transition' }, 'obs_scene')).toEqual({ type: 'obs_scene', scene: 'Scene' });
    // A carried name still passes the checks the editor shows.
    const switched = switchObsOperation({ type: 'obs_scene_collection', name: 'Stream deck' }, 'obs_profile');
    expect(obsNameIssue((switched as { name: string }).name, 'profile')).toBeNull();
  });
});

describe('obs input volume', () => {
  it('offers a step with a valid default and a label naming the input and level', () => {
    expect(stepActionTypes.map(([type]) => type)).toContain('obs_set_input_volume');
    const action = defaultStepAction('obs_set_input_volume') as { input: string; percent: number };
    expect(obsNameIssue(action.input, 'input')).toBeNull();
    expect(action.percent).toBe(100);
    expect(actionLabel({ type: 'obs_set_input_volume', input: 'Mic', percent: 40 })).toBe('Set OBS input “Mic” to 40%');
    expect(actionLabel({ type: 'obs_input_volume_from_position', input: 'Mic' })).toBe('Follow fader position for OBS input “Mic”');
    expect(obsNameIssue('', 'input')).toContain('OBS input name');
  });

  it('keeps the input name when switching operation to another that has one', () => {
    expect(switchObsOperation({ type: 'obs_set_input_volume', input: 'Desktop', percent: 20 }, 'obs_set_input_volume'))
      .toEqual({ type: 'obs_set_input_volume', input: 'Desktop', percent: 100 });
    expect(switchObsOperation({ type: 'obs_scene', scene: 'Live' }, 'obs_set_input_volume'))
      .toEqual({ type: 'obs_set_input_volume', input: 'Mic/Aux', percent: 100 });
  });

  it('recognises both fader templates', () => {
    expect(isFaderTemplate({ type: 'volume_from_position' })).toBe(true);
    expect(isFaderTemplate({ type: 'obs_input_volume_from_position' })).toBe(true);
    expect(isFaderTemplate({ type: 'mic_input_volume_from_position' })).toBe(true);
    expect(actionLabel({ type: 'mic_input_volume_from_position' })).toBe('Follow fader position for microphone volume');
    expect(isFaderTemplate({ type: 'obs_set_input_volume' })).toBe(false);
    expect(isFaderTemplate(null)).toBe(false);
  });
});
