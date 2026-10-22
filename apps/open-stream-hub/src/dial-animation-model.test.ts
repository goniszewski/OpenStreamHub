import { describe, expect, it } from 'vitest';
import type { DialAnimation } from './contracts';
import { animationRate, compareVersionsStorageKey, readCompareVersionsSetting, writeCompareVersionsSetting, animationRateLabel, averageFrameLabel, defaultDialPreset, dialPresets, frameBytes, sameAnimation, stepIndex, inUseElsewhere } from './dial-animation-model';

// A base64 string that decodes to `bytes` bytes (no padding when divisible by 3).
const frame = (bytes: number, delay_ms: number, fill = 'A') => ({ jpeg_base64: fill.repeat(Math.ceil(bytes / 3) * 4), delay_ms });
const animation = (frames: ReturnType<typeof frame>[], loop_count = 0): DialAnimation => ({ frames, loop_count });

describe('dial presets', () => {
  it('are sharp, balanced and fast, from the largest frame budget to the smallest, and sharp is the start', () => {
    expect(dialPresets.map(preset => [preset.id, preset.budgetKib])).toEqual([['sharp', null], ['balanced', 20], ['fast', 8]]);
    expect(defaultDialPreset).toBe('sharp');
  });
  it('ask only for budgets the service accepts (1 through 48 KiB) that sit on its size classes', () => {
    for (const preset of dialPresets) if (preset.budgetKib !== null) expect([8, 20]).toContain(preset.budgetKib);
  });
});

describe('dial animation model', () => {
  it('measures a frame from its base64, with padding', () => {
    expect(frameBytes(frame(3000, 100))).toBe(3000);
    expect(frameBytes({ jpeg_base64: 'AA==', delay_ms: 100 })).toBe(1);
    expect(frameBytes({ jpeg_base64: 'AAA=', delay_ms: 100 })).toBe(2);
    expect(frameBytes({ jpeg_base64: 'AAAA', delay_ms: 100 })).toBe(3);
  });
  it('reads the rate over one pass', () => {
    expect(animationRate(animation([frame(1000, 70), frame(1000, 70)]))).toBeCloseTo(14.29, 2);
    expect(animationRateLabel(animation([frame(1000, 70), frame(1000, 70)]))).toBe('14 frames a second');
    expect(animationRateLabel(animation([frame(1000, 150), frame(1000, 150)]))).toBe('6.7 frames a second');
    expect(animationRate(animation([]))).toBe(0);
  });
  it('names the mean frame size', () => {
    expect(averageFrameLabel(animation([frame(6144, 66), frame(6144, 66)]))).toBe('6.0 KiB a frame');
    expect(averageFrameLabel(animation([frame(10 * 1024, 100), frame(20 * 1024, 100)]))).toBe('15 KiB a frame');
    expect(averageFrameLabel(animation([]))).toBe('0.0 KiB a frame');
  });
  it('tells animations apart by their ends and timing', () => {
    const base = animation([frame(100, 70, 'A'), frame(100, 70, 'B'), frame(100, 70, 'C')]);
    expect(sameAnimation(base, animation([frame(100, 70, 'A'), frame(100, 70, 'B'), frame(100, 70, 'C')]))).toBe(true);
    expect(sameAnimation(base, animation([frame(100, 70, 'A'), frame(100, 70, 'B'), frame(100, 70, 'D')]))).toBe(false);
    expect(sameAnimation(base, animation([frame(100, 70, 'A'), frame(100, 80, 'B'), frame(100, 70, 'C')]))).toBe(false);
    expect(sameAnimation(base, animation([frame(100, 70, 'A'), frame(100, 70, 'C')]))).toBe(false);
    expect(sameAnimation(base, { ...base, loop_count: 2 })).toBe(false);
    expect(sameAnimation(base, undefined)).toBe(false);
    expect(sameAnimation(animation([]), animation([]))).toBe(false);
  });
});

describe('inUseElsewhere', () => {
  it('points back to the version in use only while another one is shown', () => {
    expect(inUseElsewhere(0, 1)).toBe(0);
    expect(inUseElsewhere(2, 0)).toBe(2);
    expect(inUseElsewhere(1, 1)).toBe(-1);
    expect(inUseElsewhere(-1, 1)).toBe(-1);
  });
});

describe('stepIndex', () => {
  it('moves one version either way and wraps round the ends', () => {
    expect(stepIndex(0, 1, 3)).toBe(1);
    expect(stepIndex(2, 1, 3)).toBe(0);
    expect(stepIndex(0, -1, 3)).toBe(2);
    expect(stepIndex(1, -1, 3)).toBe(0);
    expect(stepIndex(0, 1, 0)).toBe(0);
  });
});

describe('compare versions setting', () => {
  it('is on unless it was turned off, and survives broken storage', () => {
    const store = (value: string | null) => ({ getItem: () => value });
    expect(readCompareVersionsSetting(store(null))).toBe(true);
    expect(readCompareVersionsSetting(store('on'))).toBe(true);
    expect(readCompareVersionsSetting(store('off'))).toBe(false);
    expect(readCompareVersionsSetting(null)).toBe(true);
    expect(readCompareVersionsSetting({ getItem: () => { throw new Error('blocked'); } })).toBe(true);
  });
  it('is written as on or off under its own key, and a blocked write is ignored', () => {
    const written: string[][] = [];
    const storage = { setItem: (key: string, value: string) => { written.push([key, value]); } };
    writeCompareVersionsSetting(storage, false);
    writeCompareVersionsSetting(storage, true);
    expect(written).toEqual([[compareVersionsStorageKey, 'off'], [compareVersionsStorageKey, 'on']]);
    expect(() => writeCompareVersionsSetting({ setItem: () => { throw new Error('blocked'); } }, false)).not.toThrow();
    expect(() => writeCompareVersionsSetting(null, true)).not.toThrow();
  });
});
