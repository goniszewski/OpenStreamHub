import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Action } from './contracts';
import {
  defaultMultiAction, defaultProfileAction, defaultStepAction, defaultTimerAction, keyLabel, maxObsSceneBytes, maxTimerLabelBytes,
  namedKeys, obsOutputOpLabels, obsReplayBufferOpLabels, obsSceneIssue, profileDirectionLabels, stepActionTypes, textIssue, timerDuration, timerLabelIssue, timerSeconds,
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
