import { describe, expect, it } from 'vitest';
import { artworkErrorNotice, friendlyArtworkError, genericArtworkError } from './artwork-error-model';

describe('friendlyArtworkError', () => {
  it.each([
    ['Only static PNG, JPEG and BMP source images are supported', 'PNG, JPEG or BMP'],
    ['Animated PNG input is unsupported; supply one static image', 'PNG, JPEG or BMP'],
    ['The image is larger than 8 MiB', 'too large'],
    ['Dial animation source must contain 1 byte through 8 MiB', 'too large'],
    ['Source image dimensions must be between 1 and 4096 on each axis', 'too large'],
    ['Decoded source image exceeds the 64 MiB pixel-buffer limit', 'too large'],
    ['GIF canvas must be 1 through 512 pixels on each axis', 'too large'],
    ['GIF exceeds 32 MiB cumulative RGBA work', 'too large'],
    ['A dial animation holds at most 30 frames; this GIF has 45', 'at most 30 frames'],
    ['A key animation holds at most 120 frames; this GIF has 200', 'at most 120 frames'],
    ['GIF exceeds 120 frames', 'too many frames'],
    ['GIF frame delay exceeds 60 seconds', '60 seconds or less'],
    ['A dial animation needs a GIF', 'Choose a GIF'],
    ['Invalid GIF signature', 'GIF could not be read'],
    ['Could not decode GIF frame: boom', 'GIF could not be read'],
    ['Prepared dial animation exceeds 512 KiB even at JPEG quality 10', 'too detailed'],
    ['Prepared frame exceeds 64 KiB even at JPEG quality 10', 'too detailed'],
    ['The image data is not valid base64', 'Choose the file again'],
    ['JPEG source is truncated or missing a marker', 'could not be read'],
    ['Could not decode source image within limits: Format error', 'could not be read'],
    // A too-large PNG or JPEG fails in the decoder's own limit check, before ours; it is too large, not damaged.
    ['Could not read PNG header within limits: Image size exceeds limit', 'too large'],
    ['Could not decode source image within limits: Memory limit exceeded', 'too large'],
  ])('maps %s', (raw, expected) => {
    const message = friendlyArtworkError(raw);
    expect(message).toContain(expected);
    expect(message).not.toBe(genericArtworkError);
  });
  it('keeps the raw text as detail for mapped and unknown errors', () => {
    expect(artworkErrorNotice('GIF contains no frames').detail).toBe('GIF contains no frames');
    expect(artworkErrorNotice(new Error('os error 13: permission denied'))).toEqual({ message: genericArtworkError, detail: 'os error 13: permission denied' });
  });
  it('falls back to the generic text for unknown values', () => {
    expect(friendlyArtworkError({ weird: true })).toBe(genericArtworkError);
    expect(artworkErrorNotice('').detail).toBeUndefined();
  });
});
