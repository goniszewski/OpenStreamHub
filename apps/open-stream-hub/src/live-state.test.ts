import { describe, expect, it } from 'vitest';
import type { DeviceInputPayload, Gesture, ServiceModule } from './contracts';
import { emptyLive, reduceLive, resetLive } from './live-state';

const input = (module: ServiceModule, control: number, gesture: Gesture, value?: DeviceInputPayload['event']['value'], at = 1): DeviceInputPayload =>
  ({ feed_id: 'f', module, serial_number: null, event: { timestamp_micros: at, control, gesture, ...(value ? { value } : {}) } });

describe('reduceLive', () => {
  it('scales fader positions to 0..100', () => {
    const state = reduceLive(emptyLive, input('m005s', 2, 'position_changed', { type: 'position', raw: 512, maximum: 1023 }, 9));
    expect(state.m005s?.faders[2]).toBe(50);
    expect(state.m005s?.lastEventAt).toBe(9);
    expect(reduceLive(state, input('m005s', 2, 'position_changed', { type: 'position', raw: 2000, maximum: 1000 })).m005s?.faders[2]).toBe(100);
  });
  it('tracks pressed keys sorted, and releases', () => {
    let state = reduceLive(emptyLive, input('m4315', 7, 'pressed'));
    state = reduceLive(state, input('m4315', 2, 'pressed'));
    expect(state.m4315?.keysDown).toEqual([2, 7]);
    state = reduceLive(state, input('m4315', 7, 'released'));
    expect(state.m4315?.keysDown).toEqual([2]);
  });
  it('does not change press state for short/long press', () => {
    const state = reduceLive(emptyLive, input('m131e', 0, 'long_press'));
    expect(state.m131e?.keysDown).toEqual([]);
  });
  it('rotates by 15 degrees and wraps within (-360, 360)', () => {
    let state = emptyLive;
    for (let i = 0; i < 25; i++) state = reduceLive(state, input('m003k', 0, 'clockwise'));
    expect(state.m003k?.angles[0]).toBe(15);
    state = reduceLive(emptyLive, input('m131e', 0, 'left'));
    expect(state.m131e?.angles[0]).toBe(-15);
    state = reduceLive(state, input('m131e', 0, 'right'));
    expect(state.m131e?.angles[0]).toBe(0);
  });
  it('ignores out-of-range and malformed events', () => {
    expect(reduceLive(emptyLive, input('m003k', 3, 'pressed'))).toBe(emptyLive);
    expect(reduceLive(emptyLive, input('m003k', -1, 'pressed'))).toBe(emptyLive);
    expect(reduceLive(emptyLive, input('m005s', 0, 'position_changed'))).toBe(emptyLive);
    expect(reduceLive(emptyLive, input('m005s', 0, 'position_changed', { type: 'position', raw: 1, maximum: 0 }))).toBe(emptyLive);
    expect(reduceLive(emptyLive, { module: 'nope' })).toBe(emptyLive);
    expect(reduceLive(emptyLive, null)).toBe(emptyLive);
  });
  it('pulses rollers with an increasing tick', () => {
    let state = reduceLive(emptyLive, input('m032le', 1, 'up'));
    state = reduceLive(state, input('m032le', 1, 'down'));
    expect(state.m032le?.pulses[1]).toEqual({ direction: 'down', tick: 2 });
  });
  it('resets', () => { expect(resetLive()).toEqual({}); });
});
