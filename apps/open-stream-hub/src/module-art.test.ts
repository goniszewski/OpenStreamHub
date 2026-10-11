import { describe, expect, it } from 'vitest';
import { faderCapCentre, fadersDataUrl, fadersSvg, faderTrackInset, moduleGlass } from './module-art';

describe('module art', () => {
  it('maps a fader value onto the cap centre, bottom to top', () => {
    expect(faderCapCentre(0)).toBe(675);
    expect(faderCapCentre(100)).toBe(305);
    expect(faderCapCentre(50)).toBe(490);
    expect(faderCapCentre(-20)).toBe(675);
    expect(faderCapCentre(250)).toBe(305);
  });

  it('puts every cap at its own value and falls back to the resting position', () => {
    const caps = (svg: string) => [...svg.matchAll(/class="cap" transform="translate\(0 ([\d.]+)\)/g)].map(match => Number(match[1]));
    expect(caps(fadersSvg([0, 25, 50, 75, 100]))).toEqual([675, 582.5, 490, 397.5, 305]);
    expect(caps(fadersSvg([100]))).toEqual([305, 490, 490, 490, 490]);
  });

  it('prefixes every id and reference so several inline copies do not collide', () => {
    const svg = fadersSvg([50, 50, 50, 50, 50], 'a1-');
    expect(svg).toContain('id="a1-shell"');
    expect(svg).toContain('url(#a1-shell)');
    expect(svg).not.toMatch(/\bid="(?!a1-)/);
    expect(svg).not.toMatch(/url\(#(?!a1-)/);
  });

  it('serves the standalone fader art as an SVG data URL', () => {
    expect(fadersDataUrl([10, 20, 30, 40, 50])).toMatch(/^data:image\/svg\+xml,%3Csvg/);
    expect(fadersDataUrl([10, 20, 30, 40, 50])).toBe(fadersDataUrl([10, 20, 30, 40, 50]));
  });

  it('insets the cap stops by 100 of the slot\'s 570 art units', () => {
    expect(faderTrackInset).toBeCloseTo(100 / 570, 6);
  });

  it('has a glass layer only for the display modules', () => {
    expect(moduleGlass('m4315')).toBe('/modules/keys-glass.svg');
    expect(moduleGlass('m131e')).toBe('/modules/encoder-glass.svg');
    expect(moduleGlass('m005s')).toBeUndefined();
    expect(moduleGlass('m003k')).toBeUndefined();
  });
});
