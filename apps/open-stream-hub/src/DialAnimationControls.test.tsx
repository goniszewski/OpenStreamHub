import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { DialAnimation } from './contracts';
import { DialVariantPicker } from './DialAnimationControls';
import type { DialVariants } from './DialAnimationControls';

const frame = (bytes: number, delay_ms: number, fill = 'A') => ({ jpeg_base64: fill.repeat(Math.ceil(bytes / 3) * 4), delay_ms });
const sharp: DialAnimation = { frames: [frame(24_000, 150, 'A'), frame(24_000, 150, 'B')], loop_count: 0 };
const balanced: DialAnimation = { frames: [frame(14_000, 100, 'C'), frame(14_000, 100, 'D')], loop_count: 0 };
const variants: DialVariants = { sharp: { animation: sharp }, balanced: { animation: balanced }, fast: { error: 'too big' } };

const render = (current: DialAnimation | undefined, disabled = false) => renderToStaticMarkup(<DialVariantPicker variants={variants} current={current} onChoose={() => {}} disabled={disabled} />);

describe('DialVariantPicker', () => {
  it('is a carousel showing one playing version at a time, the one in use first, with its rate and frame size', () => {
    const html = render(sharp);
    expect(html).toContain('aria-roledescription="carousel"');
    expect(html.match(/<img /g)).toHaveLength(1);
    expect(html).toContain('aria-label="1 of 3: Sharp"');
    expect(html).toContain('>Sharp<span class="dial-variant-badge">In use</span>');
    expect(html).toContain('1 of 3');
    expect(html).toContain('6.7 frames a second · 23 KiB a frame');
    expect(html).toContain('aria-label="Previous version"');
    expect(html).toContain('aria-label="Next version"');
    expect(html).not.toContain('dial-variant-elsewhere');
  });
  it('starts on the version the profile holds, and says it is in use instead of offering it', () => {
    const html = render(balanced);
    expect(html).toContain('aria-label="2 of 3: Balanced"');
    expect(html).toContain('10 frames a second · 14 KiB a frame');
    expect(html).toContain('>In use</span>');
    expect(html).not.toContain('>Use Balanced<');
  });
  it('can be disabled as a whole', () => {
    const html = render(sharp, true);
    expect(html.match(/disabled=""/g)?.length).toBeGreaterThanOrEqual(2);
  });
});
