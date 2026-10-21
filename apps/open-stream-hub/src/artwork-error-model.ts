import { errorMessage } from './ui-model';

// Backend picture errors are written for logs. This turns the known ones into
// what went wrong and what to do; anything else gets a short generic line. The
// raw text is kept as `detail` (shown as a tooltip) so it is never lost.
// Rules match stable substrings and the first match wins.
const rules: Array<[match: RegExp, text: (found: RegExpMatchArray) => string]> = [
  [/Only static PNG|Animated PNG/i, () => 'This file type is not supported. Choose a still PNG, JPEG or BMP picture (animated PNGs do not work).'],
  [/animation holds at most (\d+) frames/i, found => `This GIF has too many frames. Choose a GIF with at most ${found[1]} frames.`],
  [/GIF exceeds \d+ frames/i, () => 'This GIF has too many frames. Choose a GIF with fewer frames.'],
  [/animation exceeds \d+ KiB|Prepared frame exceeds|artwork frame exceeds|even at JPEG quality/i, () => 'This picture is too detailed for the screen. Choose a simpler picture, or for a GIF one with fewer or smaller frames.'],
  [/larger than \d+ MiB|must contain 1 byte through|pixel-buffer limit|dimensions must be between|exceeds limit|Memory limit exceeded|GIF canvas|GIF exceeds/i, () => 'This picture is too large. Choose a smaller picture or one with fewer pixels.'],
  [/delay exceeds/i, () => 'This GIF has a frame that stays too long. Choose a GIF whose frames last 60 seconds or less.'],
  [/needs a GIF|requires the (dial|keypad) target/i, () => 'This picture cannot be used here. Choose a GIF for an animation, otherwise a PNG, JPEG or BMP.'],
  [/GIF/i, () => 'This GIF could not be read. Choose a different GIF or save it again.'],
  [/not valid base64|Choose an image first/i, () => 'The picture could not be loaded. Choose the file again.'],
  [/JPEG source|Could not (read|identify|decode|inspect|apply|encode)/i, () => 'This picture could not be read. It may be damaged; choose a different PNG, JPEG or BMP file.'],
];

export const genericArtworkError = 'The picture could not be prepared. Choose a different PNG, JPEG or BMP picture.';

export type ArtworkNotice = { message: string; detail?: string };

export function artworkErrorNotice(error: unknown): ArtworkNotice {
  const raw = errorMessage(error).trim();
  for (const [match, text] of rules) {
    const found = raw.match(match);
    if (found) return { message: text(found), detail: raw };
  }
  return { message: genericArtworkError, detail: raw || undefined };
}

/** Plain-language text for a backend picture error. */
export const friendlyArtworkError = (error: unknown) => artworkErrorNotice(error).message;
