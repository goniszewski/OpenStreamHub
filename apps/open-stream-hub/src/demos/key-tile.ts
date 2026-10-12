import type { Tile, Tone } from './tictactoe-game';

// A key is 72 by 72 pixels. Pictures are drawn at twice that and fitted down by the app's own key-image path.
export const tileSize = 144;

const palette: Record<Tone, { fill: string; ink: string; edge: string }> = {
  blank: { fill: '#000000', ink: '#000000', edge: '#000000' },
  empty: { fill: '#16161c', ink: '#3a3a46', edge: '#2a2a33' },
  x: { fill: '#1b1630', ink: '#c9b6ff', edge: '#4b3a8c' },
  o: { fill: '#122a2a', ink: '#8be0d6', edge: '#2f6f69' },
  last: { fill: '#3a2f12', ink: '#ffd36b', edge: '#a07d1c' },
  win: { fill: '#12331c', ink: '#7dff9a', edge: '#2fae52' },
  button: { fill: '#26262e', ink: '#f2f2f7', edge: '#55556a' },
  score: { fill: '#16161c', ink: '#f2f2f7', edge: '#2a2a33' },
  fill: { fill: '#000000', ink: '#000000', edge: '#000000' },
};

// The part of a canvas context the drawing uses, so it can be checked without a canvas.
export interface Drawing {
  fillStyle: string; strokeStyle: string; lineWidth: number; lineCap: string; font: string; textAlign: string; textBaseline: string;
  fillRect(x: number, y: number, w: number, h: number): void;
  strokeRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void; moveTo(x: number, y: number): void; lineTo(x: number, y: number): void;
  arc(x: number, y: number, r: number, from: number, to: number): void; stroke(): void;
  fillText(text: string, x: number, y: number): void;
}

function drawMark(ctx: Drawing, mark: 'X' | 'O', ink: string) {
  ctx.strokeStyle = ink;
  ctx.lineWidth = 14;
  ctx.lineCap = 'round';
  ctx.beginPath();
  if (mark === 'O') ctx.arc(tileSize / 2, tileSize / 2, 38, 0, Math.PI * 2);
  else { ctx.moveTo(34, 34); ctx.lineTo(110, 110); ctx.moveTo(110, 34); ctx.lineTo(34, 110); }
  ctx.stroke();
}

export function drawTile(ctx: Drawing, tile: Tile) {
  const { fill, ink, edge } = palette[tile.tone];
  ctx.fillStyle = tile.tone === 'fill' ? tile.colour ?? fill : fill;
  ctx.fillRect(0, 0, tileSize, tileSize);
  if (tile.tone === 'blank' || tile.tone === 'fill') return;
  ctx.strokeStyle = edge;
  ctx.lineWidth = 4;
  ctx.strokeRect(4, 4, tileSize - 8, tileSize - 8);
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  if (tile.tone === 'score') {
    ctx.fillStyle = ink;
    ctx.font = 'bold 30px sans-serif';
    ctx.fillText(tile.mark ?? '', tileSize / 2, 40);
    ctx.font = 'bold 56px sans-serif';
    ctx.fillText(tile.text ?? '', tileSize / 2, 92);
  } else if (tile.mark && tile.tone !== 'empty') {
    drawMark(ctx, tile.mark, ink);
  } else if (tile.text) {
    ctx.fillStyle = ink;
    ctx.font = 'bold 34px sans-serif';
    ctx.fillText(tile.text, tileSize / 2, tile.sub ? 56 : 72);
    if (tile.sub) { ctx.font = '26px sans-serif'; ctx.fillText(tile.sub, tileSize / 2, 92); }
  }
}

// The picture of a tile as a base64 PNG, drawn on a canvas the page creates. Null without one (no document).
export function tilePng(tile: Tile, create: () => HTMLCanvasElement | null = () => typeof document === 'undefined' ? null : document.createElement('canvas')): string | null {
  const canvas = create();
  if (!canvas) return null;
  canvas.width = tileSize;
  canvas.height = tileSize;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  drawTile(ctx as unknown as Drawing, tile);
  const url = canvas.toDataURL('image/png');
  return url.startsWith('data:image/png;base64,') ? url.slice('data:image/png;base64,'.length) : null;
}
