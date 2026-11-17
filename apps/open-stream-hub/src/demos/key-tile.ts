import type { Arrow, Tile, Tone } from './tictactoe-game';

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
  mole: { fill: '#2a1d10', ink: '#c08a4f', edge: '#7a5428' },
  hit: { fill: '#12331c', ink: '#7dff9a', edge: '#2fae52' },
  miss: { fill: '#3a1414', ink: '#ff8a8a', edge: '#b03a3a' },
  value: { fill: '#2e808c', ink: '#fafafc', edge: '#000000' },
  // Words that only tell something: no frame, so they do not look like something to press.
  label: { fill: '#000000', ink: '#c8c8d4', edge: '#000000' },
  // The one key that does something while a message shows.
  action: { fill: '#1d6b3c', ink: '#ffffff', edge: '#5fe39a' },
};

// A bold size for a number: `large`, or smaller when its digits (each about 0.6 of the size wide) would not fit.
export const numberFont = (text: string, large: number) => `bold ${Math.min(large, Math.floor((tileSize - 24) / (0.6 * Math.max(1, text.length))))}px sans-serif`;

// Dark ink on a light fill, by perceived brightness.
export function inkOn(fill: string): string {
  const value = Number.parseInt(fill.replace('#', ''), 16);
  const [r, g, b] = [(value >> 16) & 255, (value >> 8) & 255, value & 255];
  return 0.3 * r + 0.59 * g + 0.11 * b > 170 ? '#181820' : '#fafafc';
}

// The part of a canvas context the drawing uses, so it can be checked without a canvas.
export interface Drawing {
  fillStyle: string; strokeStyle: string; lineWidth: number; lineCap: string; font: string; textAlign: string; textBaseline: string;
  fillRect(x: number, y: number, w: number, h: number): void;
  strokeRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void; moveTo(x: number, y: number): void; lineTo(x: number, y: number): void;
  arc(x: number, y: number, r: number, from: number, to: number): void; stroke(): void; fill(): void;
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

function disc(ctx: Drawing, x: number, y: number, r: number, colour: string) {
  ctx.fillStyle = colour;
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fill();
}

// A mole peeking out of its hole: a round brown head, two eyes and a pink nose.
function drawMole(ctx: Drawing, ink: string) {
  disc(ctx, 72, 82, 46, ink);
  disc(ctx, 50, 68, 8, '#16161c');
  disc(ctx, 94, 68, 8, '#16161c');
  disc(ctx, 72, 90, 12, '#e89aa8');
}

function drawVerdict(ctx: Drawing, hit: boolean, ink: string) {
  ctx.strokeStyle = ink;
  ctx.lineWidth = 16;
  ctx.lineCap = 'round';
  ctx.beginPath();
  if (hit) { ctx.moveTo(32, 76); ctx.lineTo(62, 106); ctx.lineTo(114, 40); }
  else { ctx.moveTo(38, 38); ctx.lineTo(106, 106); ctx.moveTo(106, 38); ctx.lineTo(38, 106); }
  ctx.stroke();
}

// A small chevron in the top-left corner pointing `arrow`'s way.
export function drawArrow(ctx: Drawing, arrow: Arrow, ink: string) {
  const [x, y, r] = [30, 30, 10];
  const points: Record<Arrow, [number, number][]> = {
    up: [[x - r, y + r / 2], [x, y - r / 2], [x + r, y + r / 2]],
    down: [[x - r, y - r / 2], [x, y + r / 2], [x + r, y - r / 2]],
    left: [[x + r / 2, y - r], [x - r / 2, y], [x + r / 2, y + r]],
    right: [[x - r / 2, y - r], [x + r / 2, y], [x - r / 2, y + r]],
  };
  const [first, ...rest] = points[arrow];
  ctx.strokeStyle = ink;
  ctx.lineWidth = 5;
  ctx.lineCap = 'round';
  ctx.beginPath();
  ctx.moveTo(first[0], first[1]);
  rest.forEach(([px, py]) => ctx.lineTo(px, py));
  ctx.stroke();
}

export function drawTile(ctx: Drawing, tile: Tile) {
  const { fill, ink, edge } = palette[tile.tone];
  ctx.fillStyle = tile.tone === 'fill' ? tile.colour ?? fill : fill;
  ctx.fillRect(0, 0, tileSize, tileSize);
  if (tile.tone === 'blank' || tile.tone === 'fill') return;
  if (tile.tone === 'value') {
    // A numbered tile: its colour fills the key inside a thin black margin, with the number in the middle.
    const colour = tile.colour ?? fill;
    ctx.fillStyle = '#000000';
    ctx.fillRect(0, 0, tileSize, tileSize);
    ctx.fillStyle = colour;
    ctx.fillRect(6, 6, tileSize - 12, tileSize - 12);
    ctx.fillStyle = inkOn(colour);
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.font = numberFont(tile.text ?? '', 72);
    ctx.fillText(tile.text ?? '', tileSize / 2, tileSize / 2 + 4);
    if (tile.arrow) drawArrow(ctx, tile.arrow, inkOn(colour));
    return;
  }
  ctx.strokeStyle = edge;
  ctx.lineWidth = 4;
  ctx.strokeRect(4, 4, tileSize - 8, tileSize - 8);
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  if (tile.tone === 'mole') drawMole(ctx, ink);
  else if (tile.tone === 'hit' || tile.tone === 'miss') drawVerdict(ctx, tile.tone === 'hit', ink);
  else if (tile.tone === 'score' || (tile.tone === 'label' && tile.label !== undefined)) {
    ctx.fillStyle = ink;
    ctx.font = 'bold 30px sans-serif';
    ctx.fillText(tile.label ?? tile.mark ?? '', tileSize / 2, 40);
    ctx.font = numberFont(tile.text ?? '', 56);
    ctx.fillText(tile.text ?? '', tileSize / 2, 92);
  } else if (tile.mark && tile.tone !== 'empty') {
    drawMark(ctx, tile.mark, ink);
  } else if (tile.text) {
    ctx.fillStyle = ink;
    ctx.font = 'bold 34px sans-serif';
    ctx.fillText(tile.text, tileSize / 2, tile.sub ? 56 : 72);
    if (tile.sub) { ctx.font = '26px sans-serif'; ctx.fillText(tile.sub, tileSize / 2, 92); }
  }
  // On an empty key the ink is too dim for an arrow.
  if (tile.arrow) drawArrow(ctx, tile.arrow, tile.tone === 'empty' ? '#6a6a7c' : ink);
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
