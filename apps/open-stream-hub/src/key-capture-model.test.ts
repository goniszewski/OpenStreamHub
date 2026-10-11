import { describe, expect, it } from 'vitest';
import { chordFromEvent, chordLabel, foldRecording, heldLabel, keyCatalog, modifiersFromFlags, orderedModifiers, searchKeys } from './key-capture-model';
import type { CaptureEvent } from './key-capture-model';

const event = (overrides: Partial<CaptureEvent>): CaptureEvent => ({
  key: '', code: '', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...overrides,
});

describe('chord capture', () => {
  it('reads letters, digits and space as literals, case-folded', () => {
    expect(chordFromEvent(event({ key: 'a', code: 'KeyA' }))).toEqual({ chord: { key: 'a', modifiers: [] } });
    expect(chordFromEvent(event({ key: 'A', code: 'KeyA', shiftKey: true }))).toEqual({ chord: { key: 'a', modifiers: ['shift'] } });
    expect(chordFromEvent(event({ key: '5', code: 'Digit5' }))).toEqual({ chord: { key: '5', modifiers: [] } });
    expect(chordFromEvent(event({ key: ' ', code: 'Space' }))).toEqual({ chord: { key: ' ', modifiers: [] } });
  });

  it('reads named keys, the keypad and function keys from the code, not the key', () => {
    expect(chordFromEvent(event({ key: 'Enter', code: 'Enter' }))).toEqual({ chord: { key: 'return', modifiers: [] } });
    expect(chordFromEvent(event({ key: 'Backspace', code: 'Backspace' }))).toEqual({ chord: { key: 'delete', modifiers: [] } });
    expect(chordFromEvent(event({ key: 'ArrowLeft', code: 'ArrowLeft' }))).toEqual({ chord: { key: 'arrow_left', modifiers: [] } });
    expect(chordFromEvent(event({ key: 'F13', code: 'F13' }))).toEqual({ chord: { key: 'f13', modifiers: [] } });
    // A keypad 4 and the top-row 4 share event.key; only event.code tells them apart.
    expect(chordFromEvent(event({ key: '4', code: 'Numpad4' }))).toEqual({ chord: { key: 'keypad_4', modifiers: [] } });
    expect(chordFromEvent(event({ key: '4', code: 'Digit4' }))).toEqual({ chord: { key: '4', modifiers: [] } });
    expect(chordFromEvent(event({ key: 'Enter', code: 'NumpadEnter' }))).toEqual({ chord: { key: 'keypad_enter', modifiers: [] } });
    expect(chordFromEvent(event({ key: '+', code: 'NumpadAdd' }))).toEqual({ chord: { key: 'keypad_plus', modifiers: [] } });
  });

  it('holds back modifier-only presses and refuses keys it cannot send', () => {
    expect(chordFromEvent(event({ key: 'Shift', code: 'ShiftLeft', shiftKey: true }))).toEqual({ pending: true, held: ['shift'] });
    expect(chordFromEvent(event({ key: 'Meta', code: 'MetaLeft', metaKey: true }))).toEqual({ pending: true, held: ['command'] });
    expect(chordFromEvent(event({ key: 'CapsLock', code: 'CapsLock' }))).toEqual({ pending: true, held: [] });
    expect(chordFromEvent(event({ key: '/', code: 'Slash' }))).toEqual({ unsupported: '/' });
    expect(chordFromEvent(event({ key: 'é', code: 'KeyE' }))).toEqual({ unsupported: 'é' });
    expect(chordFromEvent(event({ key: 'Dead', code: 'Dead' }))).toEqual({ unsupported: 'Dead' });
  });

  it('collects the held modifiers in the display order', () => {
    const result = chordFromEvent(event({ key: 'p', code: 'KeyP', metaKey: true, ctrlKey: true, altKey: true, shiftKey: true }));
    expect(result).toEqual({ chord: { key: 'p', modifiers: ['command', 'control', 'option', 'shift'] } });
  });

  it('sends recognised media keys bare', () => {
    expect(chordFromEvent(event({ key: 'MediaPlayPause', code: '' }))).toEqual({ chord: { key: 'play_pause', modifiers: [] } });
  });
});

describe('chord labels', () => {
  it('spells the chord with the key label', () => {
    expect(chordLabel({ key: 'p', modifiers: ['command', 'shift'] })).toBe('Command+Shift+P');
    expect(chordLabel({ key: 'keypad_enter', modifiers: [] })).toBe('Keypad Enter');
    expect(chordLabel({ key: ' ', modifiers: [] })).toBe('Space');
    expect(chordLabel({ key: 'play_pause', modifiers: [] })).toBe('Play/Pause media key');
  });

  it('labels the held modifiers while capture waits for the key', () => {
    expect(heldLabel(['command', 'option'])).toBe('Command+Option+…');
  });

  it('orders modifiers the way the checkboxes list them, whatever the stored order', () => {
    expect(orderedModifiers(['shift', 'command'])).toEqual(['command', 'shift']);
    expect(modifiersFromFlags({ command: true, shift: true, control: false, option: false })).toEqual(['command', 'shift']);
  });
});

describe('key catalog', () => {
  it('offers every key group without pressing anything', () => {
    const groups = keyCatalog();
    expect(groups.map(group => group.id)).toEqual(['letters', 'named', 'functions', 'keypad', 'media']);
    const letters = groups[0].keys.map(entry => entry.key);
    expect(letters).toContain('a');
    expect(letters).toContain('9');
    expect(letters).toContain(' ');
    expect(groups[2].keys.map(entry => entry.key)).toEqual(Array.from({ length: 20 }, (_, index) => `f${index + 1}`));
    const keypad = groups[3].keys.map(entry => entry.key);
    expect(keypad).toContain('keypad_0');
    expect(keypad).toContain('keypad_equals');
    // Media keys are marked, so the macro editor can leave them out.
    expect(groups[4].media).toBe(true);
    expect(groups[4].keys.map(entry => entry.key)).toEqual(['play_pause', 'next_track', 'previous_track']);
  });

  it('narrows by every word of the query', () => {
    const keypad = keyCatalog().find(group => group.id === 'keypad')!;
    expect(searchKeys('enter', keypad).map(entry => entry.key)).toEqual(['keypad_enter']);
    expect(searchKeys('', keypad)).toHaveLength(keypad.keys.length);
    expect(searchKeys('nothing matches this', keypad)).toEqual([]);
  });
});

describe('foldRecording', () => {
  const key = (key: string | null, down: boolean, at_ms: number, flags = {}, key_code = 0) => ({
    key, key_code, down, at_ms, shift: false, control: false, option: false, command: false, ...flags,
  });

  it('folds down/up pairs into steps with waits and holds', () => {
    const folded = foldRecording([
      key('a', true, 500, { shift: true }, 0x00),
      key('a', false, 580),
      key('s', true, 900, { command: true }, 0x01),
      key('s', false, 1010),
    ]);
    expect(folded.dropped).toBe(0);
    expect(folded.trimmed).toBe(false);
    expect(folded.steps).toEqual([
      { key: 'a', modifiers: ['shift'], delay_before_ms: 0, hold_ms: 80 },
      { key: 's', modifiers: ['command'], delay_before_ms: 320, hold_ms: 110 },
    ]);
  });

  it('folds away modifier taps and auto-repeat, and counts unmapped keys', () => {
    const folded = foldRecording([
      key(null, true, 10, {}, 56), // the Shift key itself
      key(null, false, 60, {}, 56),
      key('a', true, 100),
      key('a', true, 150), // auto-repeat
      key('a', false, 180),
      key(null, true, 200, {}, 99), // a key no macro can hold
      key(null, false, 240, {}, 99),
      key('b', true, 300),
      key('b', false, 360),
      key('c', true, 500), // still held at the end
    ]);
    expect(folded.steps).toEqual([
      { key: 'a', modifiers: [], delay_before_ms: 0, hold_ms: 80 },
      { key: 'b', modifiers: [], delay_before_ms: 120, hold_ms: 60 },
    ]);
    expect(folded.dropped).toBe(2);
  });

  it('caps waits and holds and says so', () => {
    const folded = foldRecording([
      key('a', true, 0),
      key('a', false, 10),
      key('b', true, 20000),
      key('b', false, 20100),
    ]);
    expect(folded.trimmed).toBe(true);
    expect(folded.steps[1].delay_before_ms).toBe(10000);
  });

  it('keeps the steps that fit inside the total duration and drops the tail', () => {
    const events = [key('a', true, 0), key('a', false, 10)];
    for (let index = 1; index <= 4; index += 1) {
      events.push(key('a', true, index * 10000), key('a', false, index * 10000 + 10));
    }
    const folded = foldRecording(events);
    // 0 + (10 + 9990) + (10 + 9990) + cut: the fourth 10 s gap does not fit.
    expect(folded.trimmed).toBe(true);
    expect(folded.steps).toHaveLength(3);
    expect(folded.dropped).toBe(2);
  });
});
