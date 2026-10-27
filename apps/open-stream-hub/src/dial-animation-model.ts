import type { DialAnimation, DialAnimationFrame } from './contracts';

// The three ways a GIF is prepared for the dial, by how large a frame may be. A frame must stay up at least as long as
// it takes to send, which grows with its size (the service's DialAnimation::min_delay_ms_for: 66 ms up to 8 KiB, 100 ms
// up to 20 KiB, 150 ms above), so a smaller budget buys a faster animation at the cost of a rougher picture.
export type DialPreset = 'sharp' | 'balanced' | 'fast';
export interface DialPresetInfo { id: DialPreset; label: string; budgetKib: number | null; help: string }
export const dialPresets: readonly DialPresetInfo[] = [
  { id: 'sharp', label: 'Sharp', budgetKib: null, help: 'The best picture. Large frames take longer to send, so they play slower.' },
  { id: 'balanced', label: 'Balanced', budgetKib: 20, help: 'Frames up to 20 KiB, shown for 100 ms or more.' },
  { id: 'fast', label: 'Fast', budgetKib: 8, help: 'Frames up to 8 KiB, shown for 66 ms or more. A rougher picture.' },
];
export const defaultDialPreset: DialPreset = 'sharp';

export const frameBytes = (frame: DialAnimationFrame): number => {
  const text = frame.jpeg_base64;
  return Math.floor(text.length * 3 / 4) - (text.endsWith('==') ? 2 : text.endsWith('=') ? 1 : 0);
};

// Frames a second over one pass of the animation.
export const animationRate = (animation: DialAnimation): number => {
  const total = animation.frames.reduce((sum, frame) => sum + frame.delay_ms, 0);
  return total > 0 ? animation.frames.length * 1000 / total : 0;
};
export const animationRateLabel = (animation: DialAnimation): string => {
  const rate = animationRate(animation);
  return `${rate >= 10 ? rate.toFixed(0) : rate.toFixed(1)} frames a second`;
};

// The mean frame size, as KiB text.
export const averageFrameLabel = (animation: DialAnimation): string => {
  const kib = animation.frames.reduce((sum, frame) => sum + frameBytes(frame), 0) / Math.max(1, animation.frames.length) / 1024;
  return `${kib < 10 ? kib.toFixed(1) : Math.round(kib)} KiB a frame`;
};

// Whether two prepared animations are the same one (the first and last frame and the timing; the frames are large).
export const sameAnimation = (a: DialAnimation | undefined, b: DialAnimation | undefined): boolean => {
  if (!a || !b || a.frames.length !== b.frames.length || a.loop_count !== b.loop_count || a.frames.length === 0) return false;
  const last = a.frames.length - 1;
  return a.frames[0].jpeg_base64 === b.frames[0].jpeg_base64 && a.frames[last].jpeg_base64 === b.frames[last].jpeg_base64
    && a.frames.every((frame, index) => frame.delay_ms === b.frames[index].delay_ms);
};

// The next version to look at: one step either way, wrapping round the ends.
// The version the profile holds when it is not the one being looked at; -1 means none of the three is in use.
export const inUseElsewhere = (usedIndex: number, shown: number): number => usedIndex >= 0 && usedIndex !== shown ? usedIndex : -1;

export const stepIndex = (index: number, delta: number, count: number): number => count <= 0 ? 0 : (((index + delta) % count) + count) % count;

// "Compare versions of a GIF" is on unless it was turned off: choosing a GIF then prepares all three versions to look at
// (three times the work); off prepares the one version in use.
export const compareVersionsStorageKey = 'open-stream-hub.dial-compare-versions';
export function readCompareVersionsSetting(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(compareVersionsStorageKey) !== 'off'; } catch { return true; }
}
export function writeCompareVersionsSetting(storage: Pick<Storage, 'setItem'> | null, on: boolean) {
  try { storage?.setItem(compareVersionsStorageKey, on ? 'on' : 'off'); } catch { /* the toggle still works for this session */ }
}

