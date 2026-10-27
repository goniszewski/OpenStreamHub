// What an M4315 key shows when the app draws it: a pixel-for-pixel port of the pictures the service makes in
// crates/cooltek-sdk-cli/src/tile_artwork.rs (an action tile: its icon above its label) and display_artwork.rs (the
// folder and the Back key), so the editor's preview and the module agree. The shapes follow the same pixel-centre
// rules; `fixtures/key-faces.json` holds a fingerprint of every picture the Rust side draws and key-face.test.ts holds
// this port to it. Only JPEG compression, which the module adds, is not reproduced.

import type { BindingAction } from './contracts';
import { glyph } from './dial-face';

export const keyFaceSide = 72;
// The service computes in 32-bit floats; a threshold that is not exact in binary is rounded the same way here, so a
// pixel that sits right on an edge falls on the same side.
const f32 = Math.fround;
type Colour = readonly [number, number, number];
type Point = readonly [number, number];
// Three bytes per pixel, as the service's RGB image.
export type KeyFace = Uint8Array;

const BACKGROUND: Colour = [36, 52, 78];
const WHITE: Colour = [255, 255, 255];
const BLACK: Colour = [0, 0, 0];
const FOLDER_BACKGROUND: Colour = [24, 24, 24];
const FOLDER: Colour = [214, 150, 40];
const BADGE: Colour = [40, 110, 200];

export type TileIcon = 'generic' | 'volume' | 'mute' | 'play' | 'next' | 'previous' | 'stop' | 'screenshot' | 'undo' | 'redo'
  | 'timer' | 'web' | 'application' | 'text' | 'sequence' | 'profile' | 'obs';

// Mirrors TileIcon::of in crates/cooltek-sdk-service/src/display_sync.rs.
export function tileIconOf(action?: BindingAction): TileIcon {
  if (!action) return 'generic';
  switch (action.type) {
    case 'volume': case 'set_volume': case 'volume_from_position': case 'mic_input_volume_from_position': return 'volume';
    case 'toggle_mute': return 'mute';
    case 'media': return ({ play_pause: 'play', next_track: 'next', previous_track: 'previous', stop: 'stop' } as const)[action.command];
    case 'system': return ({ undo: 'undo', redo: 'redo', screenshot: 'screenshot' } as const)[action.command];
    case 'open_url': return 'web';
    case 'launch_app': return 'application';
    case 'text': return 'text';
    case 'multi_action': case 'conditional': case 'macro': return 'sequence';
    case 'profile': return 'profile';
    case 'timer': return 'timer';
    case 'shortcut': return 'generic';
    default: return action.type.startsWith('obs_') ? 'obs' : 'generic';
  }
}

const blank = (colour: Colour): KeyFace => {
  const image = new Uint8Array(keyFaceSide * keyFaceSide * 3);
  for (let at = 0; at < image.length; at += 3) image.set(colour, at);
  return image;
};

const put = (image: KeyFace, x: number, y: number, colour: Colour) => {
  if (x >= 0 && y >= 0 && x < keyFaceSide && y < keyFaceSide) image.set(colour, (y * keyFaceSide + x) * 3);
};

// Fills the pixels whose centre satisfies `inside`, within a box.
function paint(image: KeyFace, from: Point, to: Point, colour: Colour, inside: (x: number, y: number) => boolean) {
  for (let y = Math.floor(from[1]); y <= Math.ceil(to[1]); y += 1) {
    for (let x = Math.floor(from[0]); x <= Math.ceil(to[0]); x += 1) {
      if (inside(x + 0.5, y + 0.5)) put(image, x, y, colour);
    }
  }
}

const rect = (image: KeyFace, x: number, y: number, width: number, height: number, colour: Colour) =>
  paint(image, [x, y], [x + width, y + height], colour, (px, py) => px >= x && px < x + width && py >= y && py < y + height);

const disc = (image: KeyFace, [cx, cy]: Point, radius: number, colour: Colour) =>
  paint(image, [cx - radius, cy - radius], [cx + radius, cy + radius], colour, (px, py) => Math.hypot(px - cx, py - cy) <= radius);

// A ring between two radii, limited to the pixels `keep` accepts (relative to the centre).
function ring(image: KeyFace, [cx, cy]: Point, inner: number, outer: number, colour: Colour, keep: (dx: number, dy: number) => boolean) {
  paint(image, [cx - outer, cy - outer], [cx + outer, cy + outer], colour, (px, py) => {
    const distance = Math.hypot(px - cx, py - cy);
    return distance >= inner && distance <= outer && keep(px - cx, py - cy);
  });
}

function triangle(image: KeyFace, a: Point, b: Point, c: Point, colour: Colour) {
  const side = (p: Point, q: Point, r: Point) => (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
  paint(image, [Math.min(a[0], b[0], c[0]), Math.min(a[1], b[1], c[1])], [Math.max(a[0], b[0], c[0]), Math.max(a[1], b[1], c[1])], colour, (px, py) => {
    const p: Point = [px, py];
    const [d1, d2, d3] = [side(a, b, p), side(b, c, p), side(c, a, p)];
    return !((d1 < 0 || d2 < 0 || d3 < 0) && (d1 > 0 || d2 > 0 || d3 > 0));
  });
}

// A straight stroke with round ends.
function stroke(image: KeyFace, a: Point, b: Point, half: number, colour: Colour) {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
  const length = dx * dx + dy * dy;
  paint(image, [Math.min(a[0], b[0]) - half, Math.min(a[1], b[1]) - half], [Math.max(a[0], b[0]) + half, Math.max(a[1], b[1]) + half], colour, (px, py) => {
    const along = length === 0 ? 0 : Math.min(1, Math.max(0, ((px - a[0]) * dx + (py - a[1]) * dy) / length));
    return Math.hypot(px - (a[0] + along * dx), py - (a[1] + along * dy)) <= half;
  });
}

function speaker(image: KeyFace, [x, y]: Point) {
  rect(image, x - 12, y - 4, 6, 8, WHITE);
  triangle(image, [x - 6, y - 4], [x + 2, y - 12], [x + 2, y + 12], WHITE);
  triangle(image, [x - 6, y - 4], [x + 2, y + 12], [x - 6, y + 4], WHITE);
}

// Draws an icon centred on `c`, about 28 pixels across.
function icon(image: KeyFace, kind: TileIcon, c: Point) {
  const [x, y] = c;
  switch (kind) {
    case 'generic':
      rect(image, x - 12, y - 9, 24, 18, WHITE);
      rect(image, x - 10, y - 7, 20, 14, BACKGROUND);
      for (let column = 0; column < 3; column += 1) rect(image, x - 7 + column * 6, y - 3, 3, 3, WHITE);
      rect(image, x - 6, y + 2, 12, 2, WHITE);
      break;
    case 'volume':
      speaker(image, [x - 3, y]);
      for (const [inner, outer] of [[5, 7], [10, 12]]) ring(image, [x - 1, y], inner, outer, WHITE, (dx, dy) => dx > 0 && Math.abs(dy) <= dx);
      break;
    case 'mute':
      speaker(image, [x - 5, y]);
      stroke(image, [x + 4, y - 6], [x + 13, y + 6], f32(1.6), WHITE);
      stroke(image, [x + 4, y + 6], [x + 13, y - 6], f32(1.6), WHITE);
      break;
    case 'play':
      triangle(image, [x - 7, y - 12], [x - 7, y + 12], [x + 11, y], WHITE);
      break;
    case 'next':
      triangle(image, [x - 11, y - 11], [x - 11, y + 11], [x + 5, y], WHITE);
      rect(image, x + 7, y - 11, 4, 22, WHITE);
      break;
    case 'previous':
      triangle(image, [x + 11, y - 11], [x + 11, y + 11], [x - 5, y], WHITE);
      rect(image, x - 11, y - 11, 4, 22, WHITE);
      break;
    case 'stop':
      rect(image, x - 10, y - 10, 20, 20, WHITE);
      break;
    case 'screenshot':
      rect(image, x - 6, y - 12, 12, 5, WHITE);
      rect(image, x - 13, y - 8, 26, 19, WHITE);
      disc(image, [x, y + 2], 6.5, BACKGROUND);
      disc(image, [x, y + 2], 3.5, WHITE);
      break;
    case 'undo': case 'redo': {
      const flip = kind === 'redo' ? -1 : 1;
      ring(image, [x, y + 4], 8, 11, WHITE, (_, dy) => dy <= 0);
      ring(image, [x, y + 4], 8, 11, WHITE, (dx, dy) => dy > 0 && dy < 4 && dx * flip < 0);
      const tip = x - flip * 9.5;
      triangle(image, [tip - 6, y + 4], [tip + 6, y + 4], [tip, y + 12], WHITE);
      break;
    }
    case 'timer':
      ring(image, [x, y + 2], 10, 13, WHITE, () => true);
      stroke(image, [x, y + 2], [x, y - 6], 1.5, WHITE);
      stroke(image, [x, y + 2], [x + 5, y + 5], 1.5, WHITE);
      rect(image, x - 3, y - 14, 6, 3, WHITE);
      break;
    case 'web':
      ring(image, c, 10, 12.5, WHITE, () => true);
      rect(image, x - 12, y - 1, 24, 2, WHITE);
      rect(image, x - 1, y - 12, 2, 24, WHITE);
      paint(image, [x - 6, y - 12], [x + 6, y + 12], WHITE, (px, py) => {
        const value = ((px - x) / 5.5) ** 2 + ((py - y) / 11.5) ** 2;
        return value >= f32(0.65) && value <= 1;
      });
      break;
    case 'application':
      rect(image, x - 13, y - 10, 26, 21, WHITE);
      rect(image, x - 11, y - 4, 22, 13, BACKGROUND);
      for (let dot = 0; dot < 3; dot += 1) disc(image, [x - 9 + dot * 4, y - 7], f32(1.1), BACKGROUND);
      break;
    case 'text':
      rect(image, x - 11, y - 11, 22, 5, WHITE);
      rect(image, x - 3, y - 11, 6, 23, WHITE);
      break;
    case 'sequence':
      for (let row = 0; row < 3; row += 1) {
        const top = y - 9 + row * 8;
        disc(image, [x - 10, top + 1.5], 2, WHITE);
        rect(image, x - 5, top, 16, 3, WHITE);
      }
      break;
    case 'profile':
      rect(image, x - 11, y - 7, 17, 3, WHITE);
      triangle(image, [x + 4, y - 12], [x + 4, y], [x + 13, y - 5.5], WHITE);
      rect(image, x - 6, y + 5, 17, 3, WHITE);
      triangle(image, [x - 4, y], [x - 4, y + 12], [x - 13, y + 6.5], WHITE);
      break;
    case 'obs':
      rect(image, x - 13, y - 3, 26, 14, WHITE);
      rect(image, x - 11, y - 1, 22, 10, BACKGROUND);
      rect(image, x - 13, y - 11, 26, 6, WHITE);
      for (let stripe = 0; stripe < 3; stripe += 1) {
        const left = x - 9 + stripe * 7;
        stroke(image, [left, y - 5], [left + 4, y - 11], f32(1.3), BACKGROUND);
      }
      break;
  }
}

function drawText(image: KeyFace, line: string, top: number, scale: number) {
  const characters = [...line];
  const advance = 6 * scale;
  let left = Math.trunc((keyFaceSide - (characters.length * advance - scale)) / 2);
  for (const character of characters) {
    glyph(character).forEach((bits, row) => {
      for (let column = 0; column < 5; column += 1) {
        if (bits & (0b10000 >> column)) rect(image, left + column * scale, top + row * scale, scale, scale, WHITE);
      }
    });
    left += advance;
  }
}

function wrapLabel(label: string, width: number, lines: number): string[] | null {
  const wrapped: string[] = [];
  for (const word of label.split(/\s+/).filter(Boolean)) {
    if ([...word].length > width) return null;
    const last = wrapped.length - 1;
    if (last >= 0 && [...wrapped[last]].length + 1 + [...word].length <= width) wrapped[last] += ` ${word}`;
    else wrapped.push(word);
  }
  return wrapped.length <= lines ? wrapped : null;
}

// A key label in the module's 5×7 font: large (two lines of up to 6 characters) when it fits that way, else small
// (three lines of up to 12), cut with a final dot. Mirrors `tile_artwork::lines_of`.
export function keyLabelLines(label: string): { lines: string[]; scale: 1 | 2 } {
  const large = wrapLabel(label, 6, 2);
  if (large) return { lines: large, scale: 2 };
  const small = wrapLabel(label, 12, 3);
  if (small) return { lines: small, scale: 1 };
  const text = label.split(/\s+/).filter(Boolean).join(' ');
  const characters = [...text].slice(0, 36);
  const lines: string[] = [];
  for (let at = 0; at < characters.length; at += 12) lines.push(characters.slice(at, at + 12).join('').trim());
  if ([...text].length > 36) lines[lines.length - 1] = `${lines[lines.length - 1].slice(0, -1)}.`;
  return { lines, scale: 1 };
}

// An action tile: its icon above its label, or a larger icon alone without a label.
export function drawActionKey(kind: TileIcon, label: string): KeyFace {
  const image = blank(BACKGROUND);
  const { lines, scale } = keyLabelLines(label);
  if (lines.length === 0) {
    icon(image, kind, [36, 36]);
    return image;
  }
  const lineHeight = 8 * scale;
  const textTop = keyFaceSide - 6 - (lines.length * lineHeight - scale);
  icon(image, kind, [36, (textTop - 4) / 2 + 1]);
  lines.forEach((line, index) => drawText(image, line, textTop + index * lineHeight, scale));
  return image;
}

const fill = (image: KeyFace, x: number, y: number, width: number, height: number, colour: Colour) => {
  for (let py = y; py < Math.min(y + height, keyFaceSide); py += 1) for (let px = x; px < Math.min(x + width, keyFaceSide); px += 1) put(image, px, py, colour);
};

// The folder silhouette a folder tile shows: a tab above a body.
export function drawFolderKey(): KeyFace {
  const image = blank(FOLDER_BACKGROUND);
  fill(image, 12, 18, 20, 8, FOLDER);
  fill(image, 12, 24, 48, 32, FOLDER);
  return image;
}

// A left arrow above an `L{depth}` badge: the Back key.
export function drawBackKey(depth: number): KeyFace {
  const image = blank(BLACK);
  for (let x = 12; x <= 32; x += 1) {
    const half = Math.floor(((x - 12) * 12) / 20);
    fill(image, x, 22 - half, 1, 2 * half + 1, WHITE);
  }
  fill(image, 32, 18, 26, 9, WHITE);
  fill(image, 16, 40, 41, 28, BADGE);
  const block = (x: number, y: number, rows: readonly number[]) => rows.forEach((bits, row) => {
    for (let column = 0; column < 5; column += 1) if (bits & (0b10000 >> column)) fill(image, x + column * 3, y + row * 3, 3, 3, WHITE);
  });
  block(20, 44, glyph('L'));
  block(38, 44, glyph(String(depth)));
  return image;
}

export const drawBlankKey = (): KeyFace => blank(BLACK);

// The same picture as canvas pixels (RGBA).
export function keyFaceRgba(face: KeyFace): Uint8ClampedArray<ArrayBuffer> {
  const out = new Uint8ClampedArray(keyFaceSide * keyFaceSide * 4);
  for (let pixel = 0; pixel < keyFaceSide * keyFaceSide; pixel += 1) {
    out.set(face.subarray(pixel * 3, pixel * 3 + 3), pixel * 4);
    out[pixel * 4 + 3] = 255;
  }
  return out;
}

// FNV-1a over the picture's bytes, as the Rust fixture test fingerprints its pictures.
export function keyFaceFingerprint(face: KeyFace): string {
  let hash = 0xcbf29ce484222325n;
  for (const byte of face) hash = ((hash ^ BigInt(byte)) * 0x100000001b3n) & 0xffffffffffffffffn;
  return hash.toString(16).padStart(16, '0');
}
