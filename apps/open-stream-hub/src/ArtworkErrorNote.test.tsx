import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ArtworkErrorNote } from './ArtworkErrorNote';
import { artworkErrorNotice } from './artwork-error-model';

describe('ArtworkErrorNote', () => {
  it('shows the message as the alert and the raw text in a details disclosure', () => {
    const html = renderToStaticMarkup(<ArtworkErrorNote notice={artworkErrorNotice('GIF exceeds 120 frames')} />);
    expect(html).toContain('<p class="field-error" role="alert">This GIF has too many frames.');
    expect(html).toContain('<details class="field-detail"><summary>Details</summary><p>GIF exceeds 120 frames</p></details>');
  });
  it('offers no details when the message is a local one', () => {
    const html = renderToStaticMarkup(<ArtworkErrorNote notice={{ message: 'The icon could not be drawn.' }} />);
    expect(html).toContain('The icon could not be drawn.');
    expect(html).not.toContain('<details');
  });
});
