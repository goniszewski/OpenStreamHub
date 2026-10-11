// Colour maths for the in-app picker: exact hex <-> HSV conversion, so a picked colour is the colour sent.
import { formatHexColour, parseHexColour } from './artwork-model';

export interface Hsv { h: number; s: number; v: number }

// h in 0-360, s and v in 0-1.
export function rgbToHsv([r, g, b]: [number, number, number]): Hsv {
  const [R, G, B] = [r / 255, g / 255, b / 255];
  const max = Math.max(R, G, B);
  const delta = max - Math.min(R, G, B);
  let h = 0;
  if (delta > 0) {
    if (max === R) h = ((G - B) / delta) % 6;
    else if (max === G) h = (B - R) / delta + 2;
    else h = (R - G) / delta + 4;
    h = (h * 60 + 360) % 360;
  }
  return { h, s: max === 0 ? 0 : delta / max, v: max };
}

export function hsvToRgb({ h, s, v }: Hsv): [number, number, number] {
  const c = v * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = v - c;
  const [r, g, b] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return [r, g, b].map(channel => Math.round((channel + m) * 255)) as [number, number, number];
}

export const hexToHsv = (hex: string): Hsv => rgbToHsv(parseHexColour(hex) ?? [255, 255, 255]);
export const hsvToHex = (hsv: Hsv): string => formatHexColour(hsvToRgb(hsv));

// Exact channel values, unlike the system picker's swatches (its "red" is #ff2600).
export const colourPresets: { name: string; hex: string }[] = [
  { name: 'Red', hex: '#ff0000' }, { name: 'Orange', hex: '#ff8000' }, { name: 'Yellow', hex: '#ffff00' },
  { name: 'Green', hex: '#00ff00' }, { name: 'Cyan', hex: '#00ffff' }, { name: 'Blue', hex: '#0000ff' },
  { name: 'Purple', hex: '#8000ff' }, { name: 'Magenta', hex: '#ff00ff' }, { name: 'White', hex: '#ffffff' },
];
