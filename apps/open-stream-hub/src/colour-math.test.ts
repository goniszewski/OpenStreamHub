import { describe, expect, it } from 'vitest';
import { colourPresets, hexToHsv, hsvToHex } from './colour-math';

describe('colour maths', () => {
  it('round-trips every preset exactly', () => {
    for (const { hex } of colourPresets) expect(hsvToHex(hexToHsv(hex))).toBe(hex);
  });
  it('keeps pure red pure, with no green', () => {
    expect(hsvToHex({ h: 0, s: 1, v: 1 })).toBe('#ff0000');
  });
  it('round-trips arbitrary colours', () => {
    for (const hex of ['#123456', '#fedcba', '#808080', '#000000', '#010203']) expect(hsvToHex(hexToHsv(hex))).toBe(hex);
  });
});
