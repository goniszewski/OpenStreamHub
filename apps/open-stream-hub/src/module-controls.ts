import type { ModuleId } from './ui-model';

export interface ImageControl {
  control: number;
  left: number;
  top: number;
  width: number;
  height: number;
  round?: boolean;
}

// Percentages refer to the complete approved artwork, including transparent margins.
// The image and its controls share one aspect-ratio stage and scale together.
export const moduleArtwork: Record<ModuleId, { aspect: number; controls: ImageControl[] }> = {
  m131e: { aspect: 1, controls: [
    { control: 0, left: 20, top: 14, width: 60, height: 56, round: true },
  ] },
  m003k: { aspect: 1, controls: [
    { control: 0, left: 23, top: 24, width: 22, height: 25, round: true },
    { control: 1, left: 54, top: 24, width: 22, height: 25, round: true },
    { control: 2, left: 38, top: 49, width: 22, height: 25, round: true },
  ] },
  m4315: { aspect: 1.5, controls: Array.from({ length: 15 }, (_, control) => ({
    control, left: 13 + (control % 5) * 15.3, top: 20 + Math.floor(control / 5) * 20,
    width: 12.2, height: 16,
  })) },
  m005s: { aspect: 2, controls: Array.from({ length: 5 }, (_, control) => ({
    control, left: 16.8 + control * 14.3, top: 29, width: 9.5, height: 38,
  })) },
  // The Windows renderer labels wire index 0 as right and index 1 as left.
  // The workspace note retains the physical-order validation boundary.
  m032le: { aspect: 8 / 3, controls: [
    { control: 0, left: 53, top: 38, width: 28, height: 17 },
    { control: 1, left: 19, top: 38, width: 28, height: 17 },
  ] },
};
