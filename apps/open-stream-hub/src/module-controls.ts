import { knobOrientations, knobRotation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import { defaultFaderValues, fadersDataUrl } from './module-art';
import type { ModuleId } from './ui-model';

export interface ImageControl {
  control: number;
  left: number;
  top: number;
  width: number;
  height: number;
  round?: boolean;
}

// Canonical M003K hit areas on knob.svg (single knob on top), covering each cap, its knurled body and its light ring.
// Cap centres: index 2 at (50, 38), index 0 at (34, 57), index 1 at (66, 57).
const knobCanonical: ImageControl[] = [
  { control: 0, left: 22.5, top: 45.5, width: 23, height: 23, round: true },
  { control: 1, left: 54.5, top: 45.5, width: 23, height: 23, round: true },
  { control: 2, left: 38.5, top: 26.5, width: 23, height: 23, round: true },
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

// The image for a module: SVG art. The 3-knob module uses the top-down art: canonical (single knob on top)
// without an orientation, or turned to match its mounting. Hit areas are never rotated. Fader caps sit at
// `faders` (0..100 per fader); a stage that tracks the live values draws the art inline instead (see ModuleArt).
export function moduleImage(module: { id: ModuleId; art: string }, orientation?: KnobOrientation, faders?: readonly number[]): { src: string; style?: { transform: string } } {
  if (module.id === 'm005s') return { src: fadersDataUrl(faders ?? defaultFaderValues) };
  if (module.id === 'm131e' || module.id === 'm4315' || module.id === 'm032le') return { src: `/modules/${module.art}.svg` };
  if (module.id !== 'm003k') return { src: `/modules/${module.art}.png` };
  return orientation ? { src: '/modules/knob.svg', style: { transform: `rotate(${knobRotation(orientation)}deg)` } } : { src: '/modules/knob.svg' };
}

// Percentages refer to the complete approved artwork, including transparent margins.
// The image and its controls share one aspect-ratio stage and scale together.
const round1 = (value: number) => Math.round(value * 10) / 10;
const keyCentres = [295, 533, 770, 1010, 1247];
const keyRows = [290, 497, 704];
const faderCentres = [310, 603, 895, 1188, 1481];

// The round screen of the encoder, as a share of encoder.svg. Display content goes here, under the glass layer.
export const dialScreen = { left: 30.1, top: 30.1, width: 39.9, height: 39.9 };

export const moduleArtwork: Record<ModuleId, { aspect: number; controls: ImageControl[] }> = {
  m131e: { aspect: 1, controls: [
    { control: 0, left: 20.4, top: 20.3, width: 59.3, height: 59.3, round: true },
  ] },
  // Layout verified on hardware (8 October 2026): with the single knob on top, wire index 2 is
  // that knob, 0 is bottom-left and 1 is bottom-right. Other mountings come from knobControls().
  m003k: { aspect: 1, controls: knobControls('single-top') },
  // keys.svg: 164 x 148 caps (of 1536 x 1024) on a 5 x 3 grid.
  m4315: { aspect: 1.5, controls: Array.from({ length: 15 }, (_, control) => ({
    control, left: round1((keyCentres[control % 5] - 82) / 15.36), top: round1((keyRows[Math.floor(control / 5)] - 73) / 10.24),
    width: 10.7, height: 14.5,
  })) },
  // faders.svg: each hit area spans its whole slot, 233 x 570 of 1774 x 887 (see faderTrackInset).
  m005s: { aspect: 2, controls: Array.from({ length: 5 }, (_, control) => ({
    control, left: round1((faderCentres[control] - 116.5) / 17.74), top: 23.1, width: 13.1, height: 64.3,
  })) },
  // rollers.svg: the two barrels (roller-0 right, roller-1 left; each has a `.flutes` group that can scroll).
  // The Windows renderer labels wire index 0 as right and index 1 as left.
  // The workspace note retains the physical-order validation boundary.
  m032le: { aspect: 8 / 3, controls: [
    { control: 0, left: 53.4, top: 33.7, width: 29.8, height: 32.6 },
    { control: 1, left: 16.8, top: 33.7, width: 29.8, height: 32.6 },
  ] },
};
