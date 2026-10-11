import { describe, expect, it } from 'vitest';
import { layoutExtent, overhangLabel, overhangOf, placementLabel, placementOf } from './base-layout';

describe('base layout', () => {
  it('names a placement by where its centre sits on the base', () => {
    expect(placementLabel(placementOf('m4315', { col: 0, row: 0 }))).toBe('Top-left');
    expect(placementLabel(placementOf('m4315', { col: 0, row: -1 }))).toBe('Top-left');
    expect(placementLabel(placementOf('m4315', { col: 0, row: 1 }))).toBe('Bottom-left');
    expect(placementLabel(placementOf('m005s', { col: 0, row: 2 }))).toBe('Bottom-left');
    expect(placementLabel(placementOf('m131e', { col: 4, row: 0 }))).toBe('Top-right');
    expect(placementLabel(placementOf('m003k', { col: 4, row: 2 }))).toBe('Bottom-right');
  });

  it('measures how far a module hangs past each edge of the base', () => {
    const keys = placementOf('m4315', { col: 0, row: -1 });
    expect(overhangOf(keys)).toEqual({ top: 1, bottom: 0, left: 0, right: 0 });
    expect(overhangLabel(overhangOf(keys))).toBe('1 row past the top edge');
    expect(overhangOf(placementOf('m4315', { col: 0, row: 0 }))).toEqual({ top: 0, bottom: 0, left: 0, right: 0 });
    expect(overhangLabel(overhangOf(placementOf('m005s', { col: 5, row: 3 })))).toBe('1 row past the bottom edge, 3 columns past the right edge');
  });

  it('widens the drawn area only as far as modules reach beyond the base', () => {
    expect(layoutExtent([placementOf('m4315', { col: 0, row: 0 })])).toEqual({ col: 0, row: 0, cols: 6, rows: 4 });
    expect(layoutExtent([placementOf('m4315', { col: 0, row: -1 }), placementOf('m005s', { col: 0, row: 2 })])).toEqual({ col: 0, row: -1, cols: 6, rows: 5 });
  });
});
