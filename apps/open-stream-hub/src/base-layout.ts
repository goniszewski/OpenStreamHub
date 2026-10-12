import type { BaseCell, ServiceModule } from './contracts';

// The MasterHUB base is 6 cells wide and 4 tall, with its USB-C edges at the top and left.
export const baseColumns = 6;
export const baseRows = 4;

// Footprints in cells (columns by rows), from the product names (for example "15xKey w/ IPS, 4x3").
const footprints: Record<ServiceModule, { cols: number; rows: number }> = {
  m4315: { cols: 4, rows: 3 },
  m005s: { cols: 4, rows: 2 },
  m032le: { cols: 4, rows: 1 },
  m131e: { cols: 2, rows: 2 },
  m003k: { cols: 2, rows: 2 },
};

export interface Placement { col: number; row: number; cols: number; rows: number }
export interface Overhang { top: number; bottom: number; left: number; right: number }

export const placementOf = (module: ServiceModule, cell: BaseCell): Placement => ({ ...cell, ...footprints[module] });

// Cells of the module that lie outside the base, per edge.
export const overhangOf = (placement: Placement): Overhang => ({
  top: Math.max(0, -placement.row),
  left: Math.max(0, -placement.col),
  bottom: Math.max(0, placement.row + placement.rows - baseRows),
  right: Math.max(0, placement.col + placement.cols - baseColumns),
});

// The row and column ranges to draw: the base plus any module parts beyond it.
export function layoutExtent(placements: Placement[], base = { cols: baseColumns, rows: baseRows }) {
  const col = Math.min(0, ...placements.map(placement => placement.col));
  const row = Math.min(0, ...placements.map(placement => placement.row));
  return {
    col, row,
    cols: Math.max(base.cols, ...placements.map(placement => placement.col + placement.cols)) - col,
    rows: Math.max(base.rows, ...placements.map(placement => placement.row + placement.rows)) - row,
  };
}

// Named by where the module's centre sits on the base.
export function placementLabel(placement: Placement) {
  const vertical = placement.row + placement.rows / 2 < baseRows / 2 ? 'Top' : 'Bottom';
  const horizontal = placement.col + placement.cols / 2 < baseColumns / 2 ? 'left' : 'right';
  return `${vertical}-${horizontal}`;
}

export function overhangLabel(overhang: Overhang) {
  const edges: [string, number][] = [['top', overhang.top], ['bottom', overhang.bottom], ['left', overhang.left], ['right', overhang.right]];
  return edges.filter(([, count]) => count > 0)
    .map(([edge, count]) => `${count} ${edge === 'left' || edge === 'right' ? 'column' : 'row'}${count === 1 ? '' : 's'} past the ${edge} edge`)
    .join(', ');
}
