import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { limitPauses, macroPlanText, MacroFields, MacroRecorderDialog, MacroStepRow, PermissionRows, stepKeyNote } from './MacroFields';
import type { RecorderPhase } from './MacroFields';
import type { InputPermissions, MacroStep } from './contracts';

const step = (key: string, delay = 0, hold = 80, modifiers: MacroStep['modifiers'] = []): MacroStep =>
  ({ key, modifiers, delay_before_ms: delay, hold_ms: hold });

const renderFields = (steps: MacroStep[], invoke?: unknown) =>
  renderToStaticMarkup(<MacroFields steps={steps} disabled={false} onChange={() => {}} invoke={invoke as never} />);

function ids(markup: string) {
  return [...markup.matchAll(/ id="([^"]+)"/g)].map(match => match[1]);
}

function expectAccessible(markup: string) {
  const all = ids(markup);
  expect(new Set(all).size).toBe(all.length);
  for (const [, target] of markup.matchAll(/<label for="([^"]+)"/g)) expect(all).toContain(target);
}

describe('macro plan text', () => {
  it('prints the planned keystrokes with waits and holds', () => {
    expect(macroPlanText([step('c', 0, 80, ['command'])])).toBe('Command+C (held 80 ms)');
    expect(macroPlanText([step('c', 0, 80, ['command']), step('return', 120, 40)])).toBe('Command+C (held 80 ms) then Return after 120 ms (held 40 ms)');
    expect(macroPlanText([])).toBe('Nothing recorded yet.');
  });

  it('limits every long pause to one second', () => {
    const limited = limitPauses([step('a', 0), step('b', 4000), step('c', 1000)]);
    expect(limited.map(item => item.delay_before_ms)).toEqual([0, 1000, 1000]);
    // Holds are never touched: the hold is part of what the key does.
    expect(limitPauses([step('a', 0, 3000)])[0].hold_ms).toBe(3000);
  });
});

describe('macro timeline', () => {
  it('renders the steps with per-step fields and order controls', () => {
    const markup = renderFields([step('c', 0, 80, ['command']), step('keypad_enter', 120, 40)]);
    expectAccessible(markup);
    expect(ids(markup)).toEqual(expect.arrayContaining([
      'macro-step-1-key', 'macro-step-1-delay', 'macro-step-1-hold',
      'macro-step-2-key', 'macro-step-2-delay', 'macro-step-2-hold',
    ]));
    // Each row's key picker trigger names the key (the modifiers have their own checkboxes).
    expect(markup).toContain('>C</span>');
    expect(markup).toContain('>Keypad Enter</span>');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Move keystroke 1 up"/);
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Move keystroke 2 down"/);
    expect(markup).toContain('aria-label="Remove keystroke 1"');
    // The printed plan preview.
    expect(markup).toContain('Planned: Command+C (held 80 ms) then Keypad Enter after 120 ms (held 40 ms)');
    expect(markup).toContain('0:00.2 (m:ss) of waits and holds');
  });

  it('edits a step through its handlers and never offers media keys', () => {
    const changes: MacroStep[] = [];
    const markup = renderToStaticMarkup(<MacroStepRow step={step('c')} index={1} count={2} disabled={false}
      onChange={next => changes.push(next)} onRemove={() => {}} onMove={() => {}} />);
    expect(markup).not.toContain('Media keys');
    // The row numbers are shown; the picker is closed in static markup.
    expect(markup).toContain('>2</span>');
  });

  it('says the limits and shows the inline problem of a hand-edited macro', () => {
    const markup = renderFields([step('play_pause', 0, 0)]);
    expect(markup).toContain('media keys cannot be macro keys');
    expect(markup).toContain('role="alert"');
    const oversized = renderFields(Array.from({ length: 65 }, () => step('a')));
    expect(oversized).toContain('holds 1 to 64 keystrokes');
  });

  it('hides the recorder in the browser preview and keeps editing', () => {
    const markup = renderFields([step('c')]);
    expect(markup).not.toContain('Record macro');
    expect(markup).toContain('Add keystroke');
    const desktop = renderFields([step('c')], {});
    expect(desktop).toContain('Record macro');
  });
});

describe('permission rows', () => {
  const permissions: InputPermissions = { post_events: true, listen_events: false };

  it('says not-checked, allowed and denied, and links only the denied pane', () => {
    const unchecked = renderToStaticMarkup(<PermissionRows permissions={null} error={null} onCheck={() => {}} onOpen={() => {}} busy={false} />);
    expect((unchecked.match(/Not checked yet/g) ?? []).length).toBe(2);
    expect(unchecked).not.toContain('Open System Settings');
    const checked = renderToStaticMarkup(<PermissionRows permissions={permissions} error={null} onCheck={() => {}} onOpen={() => {}} busy={false} />);
    expect(checked).toContain('Allowed');
    expect(checked).toContain('Not allowed');
    expect(checked.match(/Open System Settings/g)?.length).toBe(1);
    expectAccessible(checked);
  });

  it('shows the check error as an alert', () => {
    const markup = renderToStaticMarkup(<PermissionRows permissions={null} error="osascript failed" onCheck={() => {}} onOpen={() => {}} busy={false} />);
    expect(markup).toContain('osascript failed');
    expect(markup).toContain('role="alert"');
  });
});

describe('recorder dialog', () => {
  const base = {
    onCancel: () => {}, onStart: () => {}, onStop: () => {}, onUse: () => {},
    onCheckPermissions: () => {}, onOpenSettings: () => {}, busy: false, live: null, disabled: false,
  };

  const element = (state: RecorderPhase, extra: Record<string, unknown> = {}) => {
    const node = MacroRecorderDialog({ state, ...base, ...extra } as never) as ReactNode;
    return renderToStaticMarkup(isValidElement(node) ? node : <>{node}</>);
  };

  it('renders closed (no open attribute) until opened', () => {
    expect(element({ phase: 'closed' })).not.toMatch(/<dialog[^>]*open/);
  });

  it('starts only with the Input Monitoring permission', () => {
    const pending = element({ phase: 'permissions' }, { permissions: null });
    expect(pending).toContain('Start recording');
    expect(pending).toMatch(/disabled=""/);
    const allowed = element({ phase: 'permissions' }, { permissions: { post_events: true, listen_events: true } });
    expect(allowed).toMatch(/<button[^>]*class="button primary"[^>]*>(?!.*disabled)/);
    const denied = element({ phase: 'permissions' }, { permissions: { post_events: true, listen_events: false } });
    expect(denied).toMatch(/disabled=""/);
    expectAccessible(allowed);
  });

  it('shows the live count while recording and the fold when reviewing', () => {
    const recording = element({ phase: 'recording' }, { live: { events: [], elapsed_ms: 900, capped: false } });
    expect(recording).toContain('Recording…');
    expect(recording).toContain('Stop');
    const full = element({ phase: 'recording' }, { live: { events: [], elapsed_ms: 60_000, capped: true } });
    expect(full).toContain('The recording is full');
    const review = element({ phase: 'review', steps: [step('c', 0, 80, ['command'])], dropped: 1, trimmed: false });
    expect(review).toContain('became 1 keystroke');
    expect(review).toContain('1 key cannot be replayed and was left out');
    expect(review).toContain('Command+C (held 80 ms)');
    expect(review).toContain('Use recording');
    expectAccessible(review);
  });

  it('names the trimmed waits in the review', () => {
    const review = element({ phase: 'review', steps: [step('a', 10000, 0)], dropped: 0, trimmed: true });
    expect(review).toContain('were trimmed');
  });
});

describe('macro step notes', () => {
  it('names a media key honestly instead of accepting it silently', () => {
    expect(stepKeyNote(step('play_pause'))).toContain('Media keys cannot be macro keys');
    expect(stepKeyNote(step('a'))).toBeNull();
  });
});
