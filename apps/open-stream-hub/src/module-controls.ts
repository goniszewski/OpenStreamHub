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
    // Follow the metal caps in perspective, rather than enclosing the bodies and LEDs.
    { control: 0, left: 25.6, top: 24.3, width: 16.7, height: 14.4, round: true },
    { control: 1, left: 57.5, top: 24.3, width: 16.7, height: 14.4, round: true },
    { control: 2, left: 41, top: 50.1, width: 17.8, height: 15.2, round: true },
  ] },
  m4315: { aspect: 1.5, controls: Array.from({ length: 15 }, (_, control) => ({
    // The rightmost caps sit slightly farther right in the perspective artwork.
    control, left: control % 5 === 4 ? 75 : 13 + (control % 5) * 15.3,
    top: 20 + Math.floor(control / 5) * 20,
    width: control % 5 === 4 ? 11.8 : 12.2, height: 16,
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
