import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ArtworkPage } from './ArtworkPage';
import type { ArtworkSource } from './ArtworkPage';
import { offlineNote } from './artwork-model';
import prepareResult from '../fixtures/artwork-prepare-result.json';
import saveResult from '../fixtures/artwork-save-result.json';

const invoke = vi.fn() as never;
const source: ArtworkSource = { name: 'cover.png', base64: 'AAAA', mime: 'image/png', size: 2048 };
const render = (props: Partial<Parameters<typeof ArtworkPage>[0]> = {}) => renderToStaticMarkup(<ArtworkPage desktop invoke={invoke} {...props} />);

describe('ArtworkPage', () => {
  it('shows the offline note, defaults and no preview before an image is chosen', () => {
    const html = render();
    expect(html).toContain(offlineNote);
    expect(html).toContain('Drop a PNG or JPEG here');
    expect(html).toContain('accept="image/png,image/jpeg,.png,.jpg,.jpeg"');
    expect(html).toContain('Encoder dial 240×240');
    expect(html).toContain('Key 72×72');
    expect(html).toContain('Keep the whole image and fill the margins');
    expect(html).toMatch(/aria-pressed="true"[^>]*>Encoder dial/);
    expect(html).toMatch(/aria-pressed="true"[^>]*>Contain/);
    expect(html).toMatch(/aria-pressed="true"[^>]*>0°/);
    expect(html).toContain('value="#000000"');
    expect(html).not.toContain('artwork-quality');
    expect(html).toContain('Choose an image to see how it will look');
    expect(html).not.toContain('artwork-stage');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[^<]*(<svg[^>]*>.*?<\/svg>)?Export JPEG/);
    expect(html).not.toContain('needs the desktop app');
  });

  it('browser preview disables the settings but still shows the source thumbnail', () => {
    const html = render({ desktop: false, initialSource: source });
    expect(html).toContain('needs the desktop app');
    expect(html).toContain('src="data:image/png;base64,AAAA"');
    expect(html).toContain('cover.png');
    expect(html.match(/<fieldset[^>]*disabled=""/g)).toHaveLength(3);
    expect(html).toMatch(/<input id="artwork-background"[^>]*disabled=""/);
    expect(html).toMatch(/disabled=""[^>]*>(<svg[^>]*>.*?<\/svg>)?Export JPEG/);
    // The picker itself stays usable.
    expect(html).not.toMatch(/<input[^>]*aria-label="Source image file"[^>]*disabled/);
    expect(html).toContain('The prepared preview needs the desktop app.');
  });

  it('shows the prepared image in the module artwork and at actual size with its path after export', () => {
    const html = render({ initialSource: source, initialOptions: { target: 'keypad' }, initialResult: prepareResult.prepare_artwork, initialSaved: { path: '/storage/artwork/cover-keypad-20261005-120000.jpg', bytes: 1673, width: 72, height: 72 } });
    const jpeg = `data:image/jpeg;base64,${prepareResult.prepare_artwork.jpeg_base64}`;
    expect(html.split(jpeg).length - 1).toBe(2);
    expect(html).toContain('artwork-stage keypad');
    expect(html).toContain('/modules/keys.svg');
    expect(html).toContain('left:13.9%;top:21.2%;width:10.7%;height:14.5%');
    expect(html).toContain('width="72" height="72"');
    expect(html).toContain('Prepared size: 1.6 KiB');
    expect(html).toContain('JPEG exported');
    expect(html).toContain('/storage/artwork/cover-keypad-20261005-120000.jpg');
    expect(html).toContain('Copy path');
    expect(html).toContain('Nothing was sent to a device');
    expect(html).not.toMatch(/disabled=""[^>]*>(<svg[^>]*>.*?<\/svg>)?Export JPEG/);
  });

  it('uses the encoder display rectangle for the dial target', () => {
    const html = render({ initialSource: source, initialResult: { ...prepareResult.prepare_artwork, width: 240, height: 240 } });
    expect(html).toContain('/modules/encoder.svg');
    // The glass layer is over the picture, so the picture sits under it.
    expect(html.indexOf('artwork-display round')).toBeLessThan(html.indexOf('/modules/encoder-glass.svg" alt'));
    expect(html).toContain('artwork-display round');
    expect(html).toContain('left:30.1%;top:30.1%;width:39.9%;height:39.9%');
    // Same geometry as the hero: the box, then the round screen inside it (styles.css).
    expect(html).toMatch(/artwork-dial-box" style="left:30.1%[^>]*><div class="artwork-display round"/);
    expect(html).toContain('width="240" height="240"');
  });

  it('keeps the invalid colour visible and flags it', () => {
    const html = render({ initialSource: source, initialOptions: { background: 'blue' } });
    expect(html).toContain('aria-invalid="true"');
    expect(html).toContain('role="alert"');
    expect(html).toContain('value="blue"');
  });
});
