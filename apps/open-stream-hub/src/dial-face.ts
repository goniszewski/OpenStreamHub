// A pixel-faithful port of the dial faces the desktop app draws on the M131E
// screen (crates/cooltek-sdk-cli/src/dial_artwork.rs): the same 240×240
// rasterizer, palette, 5×7 glyphs and seven-segment digits, so the editor
// preview shows the face the device gets. Faces are drawn per call into a
// canvas 2D context via putImageData; the drawing is a few tens of thousands
// of pixels and stays off React state.

export const dialFaceSide = 240;

export type Rgb = readonly [number, number, number];

// The dial_artwork.rs palette.
const BLACK: Rgb = [0, 0, 0];
const WHITE: Rgb = [255, 255, 255];
const DIM: Rgb = [90, 98, 112];
const ACCENT: Rgb = [40, 140, 220];
const HAND: Rgb = [230, 120, 40];

const CENTRE = (dialFaceSide - 1) / 2;
const LINE_CHARS = 11;
const NAME_CHARS = 2 * LINE_CHARS;
const MAX_DOTS = 16;
const CPU_HIGH = 80;
const CLOCK_DIGITS_X = [25, 67, 137, 179] as const;
const CLOCK_DIGITS_Y = 85;
const DIGIT_WIDTH = 36;
const DIGIT_HEIGHT = 70;
const STROKE = 8;
const CPU_DIGIT_PITCH = DIGIT_WIDTH + 6;
const CPU_RING_INNER = 102;
const CPU_RING_OUTER = 114;

export interface DialTime { hour: number; minute: number }

/** The current local time of day, minute resolution. */
export function nowDialTime(date = new Date()): DialTime {
  return { hour: date.getHours(), minute: date.getMinutes() };
}

// The same 5×7 glyph rows as dial_artwork.rs; unknown characters map to '?'.
const GLYPHS: Record<string, readonly [number, number, number, number, number, number, number]> = {
  A: [14, 17, 17, 31, 17, 17, 17], B: [30, 17, 17, 30, 17, 17, 30], C: [14, 17, 16, 16, 16, 17, 14],
  D: [30, 17, 17, 17, 17, 17, 30], E: [31, 16, 16, 30, 16, 16, 31], F: [31, 16, 16, 30, 16, 16, 16],
  G: [14, 17, 16, 23, 17, 17, 15], H: [17, 17, 17, 31, 17, 17, 17], I: [14, 4, 4, 4, 4, 4, 14],
  J: [7, 2, 2, 2, 2, 18, 12], K: [17, 18, 20, 24, 20, 18, 17], L: [16, 16, 16, 16, 16, 16, 31],
  M: [17, 27, 21, 21, 17, 17, 17], N: [17, 17, 25, 21, 19, 17, 17], O: [14, 17, 17, 17, 17, 17, 14],
  P: [30, 17, 17, 30, 16, 16, 16], Q: [14, 17, 17, 17, 21, 18, 13], R: [30, 17, 17, 30, 20, 18, 17],
  S: [15, 16, 16, 14, 1, 1, 30], T: [31, 4, 4, 4, 4, 4, 4], U: [17, 17, 17, 17, 17, 17, 14],
  V: [17, 17, 17, 17, 17, 10, 4], W: [17, 17, 17, 21, 21, 21, 10], X: [17, 17, 10, 4, 10, 17, 17],
  Y: [17, 17, 17, 10, 4, 4, 4], Z: [31, 1, 2, 4, 8, 16, 31],
  '0': [14, 17, 19, 21, 25, 17, 14], '1': [4, 12, 4, 4, 4, 4, 14], '2': [14, 17, 1, 2, 4, 8, 31],
  '3': [31, 2, 4, 2, 1, 17, 14], '4': [2, 6, 10, 18, 31, 2, 2], '5': [31, 16, 30, 1, 1, 17, 14],
  '6': [6, 8, 16, 30, 17, 17, 14], '7': [31, 1, 2, 4, 8, 8, 8], '8': [14, 17, 17, 14, 17, 17, 14],
  '9': [14, 17, 17, 15, 1, 2, 12], ' ': [0, 0, 0, 0, 0, 0, 0], '-': [0, 0, 0, 31, 0, 0, 0],
  _: [0, 0, 0, 0, 0, 0, 31], '.': [0, 0, 0, 0, 0, 12, 12], ',': [0, 0, 0, 0, 12, 4, 8],
  ':': [0, 12, 12, 0, 12, 12, 0], '/': [0, 1, 2, 4, 8, 16, 0], '+': [0, 4, 4, 31, 4, 4, 0],
  '%': [24, 25, 2, 4, 8, 19, 3], '&': [12, 18, 20, 8, 21, 18, 13], "'": [12, 4, 8, 0, 0, 0, 0],
  '(': [2, 4, 8, 8, 8, 4, 2], ')': [8, 4, 2, 2, 2, 4, 8], '#': [10, 10, 31, 10, 31, 10, 10],
  '!': [4, 4, 4, 4, 4, 0, 4],
};

export const glyph = (character: string) => GLYPHS[character.toUpperCase()] ?? GLYPHS['?'] ?? [14, 17, 1, 2, 4, 0, 4];

// Seven-segment masks, bit 0 = a (top) through bit 6 = g (middle).
const SEGMENTS = [0b011_1111, 0b000_0110, 0b101_1011, 0b100_1111, 0b110_0110, 0b110_1101, 0b111_1101, 0b000_0111, 0b111_1111, 0b110_1111];

class DialCanvas {
  readonly pixels = new Uint8ClampedArray(dialFaceSide * dialFaceSide * 4);

  put(x: number, y: number, [r, g, b]: Rgb) {
    if (x < 0 || y < 0 || x >= dialFaceSide || y >= dialFaceSide) return;
    const at = (y * dialFaceSide + x) * 4;
    this.pixels[at] = r;
    this.pixels[at + 1] = g;
    this.pixels[at + 2] = b;
    this.pixels[at + 3] = 255;
  }

  fill(x: number, y: number, width: number, height: number, color: Rgb) {
    for (let py = Math.max(0, y); py < Math.min(y + height, dialFaceSide); py++) {
      for (let px = Math.max(0, x); px < Math.min(x + width, dialFaceSide); px++) {
        this.put(px, py, color);
      }
    }
  }

  disc(cx: number, cy: number, radius: number, color: Rgb) {
    for (let py = Math.floor(cy - radius); py <= Math.ceil(cy + radius); py++) {
      for (let px = Math.floor(cx - radius); px <= Math.ceil(cx + radius); px++) {
        const dx = px - cx, dy = py - cy;
        if (dx * dx + dy * dy <= radius * radius) this.put(px, py, color);
      }
    }
  }

  ring(inner: number, outer: number, color: Rgb) {
    for (let py = 0; py < dialFaceSide; py++) {
      for (let px = 0; px < dialFaceSide; px++) {
        const dx = px - CENTRE, dy = py - CENTRE;
        const distance = Math.sqrt(dx * dx + dy * dy);
        if (distance >= inner && distance <= outer) this.put(px, py, color);
      }
    }
  }

  // A thick line from the centre at `degrees` clockwise from 12 o'clock,
  // between radii `from` and `to`.
  spoke(degrees: number, from: number, to: number, half: number, color: Rgb) {
    const angle = (degrees * Math.PI) / 180;
    const sin = Math.sin(angle), cos = Math.cos(angle);
    const steps = Math.max(1, Math.ceil((to - from) * 2));
    for (let step = 0; step <= steps; step++) {
      const radius = from + ((to - from) * step) / steps;
      this.disc(CENTRE + sin * radius, CENTRE - cos * radius, half, color);
    }
  }

  segmentMask(x: number, y: number, mask: number, color: Rgb) {
    const [w, h, t] = [DIGIT_WIDTH, DIGIT_HEIGHT, STROKE];
    const middle = y + ((h - t) / 2);
    const rects = [
      [x, y, w, t], // a
      [x + w - t, y, t, h / 2], // b
      [x + w - t, y + h / 2, t, h - h / 2], // c
      [x, y + h - t, w, t], // d
      [x, y + h / 2, t, h - h / 2], // e
      [x, y, t, h / 2], // f
      [x, middle, w, t], // g
    ] as const;
    rects.forEach(([rx, ry, rw, rh], bit) => {
      if (mask & (1 << bit)) this.fill(rx, ry, rw, rh, color);
    });
  }

  segmentDigit(x: number, y: number, digit: number, color: Rgb) {
    this.segmentMask(x, y, SEGMENTS[digit % 10], color);
  }

  // Draws `line` with its top edge at `y`, centred, at `scale` pixels per font pixel.
  text(y: number, line: string, scale: number, color: Rgb) {
    const advance = 6 * scale;
    const width = line.length * advance - scale;
    let x = Math.trunc((dialFaceSide - width) / 2);
    for (const character of line) {
      glyph(character).forEach((bits, row) => {
        for (let column = 0; column < 5; column++) {
          if (bits & (0b10000 >> column)) this.fill(x + column * scale, y + row * scale, scale, scale, color);
        }
      });
      x += advance;
    }
  }
}

// Splits a name into at most two lines of 11 characters, preferring a space; a
// longer name is cut and ends in '.'.
/** Split for the dial name faces; exported for the preview tests. */
export function wrapDialName(name: string): string[] {
  let chars = Array.from(name.trim());
  if (chars.length > NAME_CHARS) {
    chars = chars.slice(0, NAME_CHARS);
    chars[NAME_CHARS - 1] = '.';
  }
  if (chars.length <= LINE_CHARS) return [chars.join('')];
  const head = chars.slice(0, LINE_CHARS + 1);
  const splitAt = head.lastIndexOf(' ');
  const split = splitAt > 0 && chars.length - splitAt - 1 <= LINE_CHARS ? splitAt : LINE_CHARS;
  let rest = chars.slice(split).join('').trim();
  if (rest.length > LINE_CHARS) {
    const restChars = Array.from(rest.slice(0, LINE_CHARS));
    restChars[LINE_CHARS - 1] = '.';
    rest = restChars.join('');
  }
  return [chars.slice(0, split).join('').trimEnd(), rest];
}

function nameLines(face: DialCanvas, name: string) {
  const lines = wrapDialName(name);
  const top = lines.length === 1 ? 99 : 81;
  lines.forEach((line, index) => face.text(top + index * 36, line, 3, WHITE));
}

// The function name with `n/N` above it and one dot per function below.
function drawFunction(face: DialCanvas, name: string, index: number, count: number) {
  face.ring(114, 117, ACCENT);
  face.text(44, `${index + 1}/${count}`, 2, DIM);
  nameLines(face, name);
  if (count <= MAX_DOTS) {
    const pitch = 12;
    const start = Math.trunc((dialFaceSide - (count - 1) * pitch) / 2);
    for (let dot = 0; dot < count; dot++) {
      face.disc(start + dot * pitch, 178, 3.5, dot === index ? ACCENT : DIM);
    }
  }
}

// The profile name, for function content without dial functions.
function drawProfile(face: DialCanvas, name: string) {
  face.ring(114, 117, DIM);
  face.text(44, 'PROFILE', 2, DIM);
  nameLines(face, name);
}

function drawClock(face: DialCanvas, time: DialTime, analog: boolean) {
  if (analog) {
    face.ring(112, 116, DIM);
    for (let hour = 0; hour < 12; hour++) {
      const long = hour % 3 === 0;
      face.spoke(hour * 30, long ? 92 : 100, 108, long ? 3 : 2, WHITE);
    }
    const hours = (time.hour % 12) + time.minute / 60;
    face.spoke(hours * 30, 0, 55, 5, WHITE);
    face.spoke(time.minute * 6, 0, 85, 3, HAND);
    face.disc(CENTRE, CENTRE, 7, HAND);
    return;
  }
  const digits = [Math.floor(time.hour / 10), time.hour % 10, Math.floor(time.minute / 10), time.minute % 10];
  CLOCK_DIGITS_X.forEach((x, index) => face.segmentDigit(x, CLOCK_DIGITS_Y, digits[index], WHITE));
  // Colon between the hour and minute pairs.
  face.fill(116, 103, STROKE, STROKE, ACCENT);
  face.fill(116, 129, STROKE, STROKE, ACCENT);
}

// CPU usage: `CPU` above, the percentage in seven-segment digits (`--` when
// unknown), `%` below, and a ring filled clockwise from 12 o'clock.
function drawCpu(face: DialCanvas, percent: number | null) {
  const value = percent === null ? null : Math.min(percent, 100);
  const color = value !== null && value >= CPU_HIGH ? HAND : ACCENT;
  const lit = (value ?? 0) * 3.6;
  for (let py = 0; py < dialFaceSide; py++) {
    for (let px = 0; px < dialFaceSide; px++) {
      const dx = px - CENTRE, dy = py - CENTRE;
      const distance = Math.sqrt(dx * dx + dy * dy);
      if (distance < CPU_RING_INNER || distance > CPU_RING_OUTER) continue;
      // Clockwise from 12 o'clock, in [0, 360).
      const angle = ((Math.atan2(dx, -dy) * 180) / Math.PI + 360) % 360;
      face.put(px, py, angle < lit ? color : DIM);
    }
  }
  face.text(52, 'CPU', 2, DIM);
  if (value === null) {
    for (const x of cpuDigitsX(2)) face.segmentMask(x, CLOCK_DIGITS_Y, 0b100_0000, DIM);
  } else {
    String(value).split('').forEach((digit, index) => {
      face.segmentDigit(cpuDigitsX(String(value).length)[index], CLOCK_DIGITS_Y, Number(digit), WHITE);
    });
  }
  face.text(166, '%', 3, DIM);
}

// Left edges of `cells` seven-segment cells, centred on the dial.
function cpuDigitsX(cells: number): number[] {
  const left = Math.trunc((dialFaceSide - (cells * CPU_DIGIT_PITCH - (CPU_DIGIT_PITCH - DIGIT_WIDTH))) / 2);
  return Array.from({ length: cells }, (_, cell) => left + cell * CPU_DIGIT_PITCH);
}

/** The raw RGBA pixels of a face, so tests can assert what was drawn. */
export function dialFacePixels(spec: {
  kind: 'function'; name: string; index: number; count: number;
} | {
  kind: 'profile'; name: string;
} | {
  kind: 'clock' | 'analog_clock'; time: DialTime;
} | {
  kind: 'cpu'; percent: number | null;
}) {
  const face = new DialCanvas();
  face.fill(0, 0, dialFaceSide, dialFaceSide, BLACK);
  switch (spec.kind) {
    case 'function': drawFunction(face, spec.name, spec.index, spec.count); break;
    case 'profile': drawProfile(face, spec.name); break;
    case 'clock': drawClock(face, spec.time, false); break;
    case 'analog_clock': drawClock(face, spec.time, true); break;
    case 'cpu': drawCpu(face, spec.percent); break;
  }
  return face.pixels;
}

/** Paints the given face into `context` (a 240×240 canvas). */
export function paintDialFace(context: CanvasRenderingContext2D, spec: Parameters<typeof dialFacePixels>[0]): void {
  context.putImageData(new ImageData(dialFacePixels(spec), dialFaceSide, dialFaceSide), 0, 0);
}
