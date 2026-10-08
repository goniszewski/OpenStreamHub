import { knobOrientations, knobRotation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import type { ModuleId } from './ui-model';

export interface ImageControl {
  control: number;
  left: number;
  top: number;
  width: number;
  height: number;
  round?: boolean;
}

// Canonical M003K hit areas on knob-top.svg (single knob on top), covering each cap and its knurled body.
// Cap centres: index 2 at (50, 33), index 0 at (33, 66), index 1 at (67, 66).
const knobCanonical: ImageControl[] = [
  { control: 0, left: 22, top: 55, width: 22, height: 22, round: true },
  { control: 1, left: 56, top: 55, width: 22, height: 22, round: true },
  { control: 2, left: 39, top: 22, width: 22, height: 22, round: true },
];

// Rotates the canonical areas 90 degrees clockwise per step about the square's centre,
// matching the CSS rotation applied to the image. Text inside the areas stays upright.
export function knobControls(orientation: KnobOrientation): ImageControl[] {
  const turns = Math.max(0, knobOrientations.indexOf(orientation));
  return knobCanonical.map(area => {
    let { left, top, width, height } = area;
    for (let step = 0; step < turns; step++) {
      // A clockwise quarter turn maps the centre (x, y) to (100 - y, x).
      const x = left + width / 2, y = top + height / 2;
      [width, height] = [height, width];
      left = 100 - y - width / 2;
      top = x - height / 2;
    }
    return { ...area, left, top, width, height };
  });
}

// The image for a module. The 3-knob module uses the top-down art: canonical (single knob on top)
// without an orientation, or turned to match its mounting. Hit areas are never rotated.
export function moduleImage(module: { id: ModuleId; art: string }, orientation?: KnobOrientation): { src: string; style?: { transform: string } } {
  if (module.id !== 'm003k') return { src: `/modules/${module.art}.png` };
  return orientation ? { src: '/modules/knob-top.svg', style: { transform: `rotate(${knobRotation(orientation)}deg)` } } : { src: '/modules/knob-top.svg' };
}

// Percentages refer to the complete approved artwork, including transparent margins.
// The image and its controls share one aspect-ratio stage and scale together.
export const moduleArtwork: Record<ModuleId, { aspect: number; controls: ImageControl[] }> = {
  m131e: { aspect: 1, controls: [
    { control: 0, left: 20, top: 14, width: 60, height: 56, round: true },
  ] },
  // Layout verified on hardware (8 October 2026): with the single knob on top, wire index 2 is
  // that knob, 0 is bottom-left and 1 is bottom-right. Other mountings come from knobControls().
  m003k: { aspect: 1, controls: knobControls('single-top') },
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
