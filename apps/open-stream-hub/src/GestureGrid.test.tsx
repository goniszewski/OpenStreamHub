import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { GestureGrid } from './GestureGrid';
import { keySlotUsed } from './keypad-model';

describe('GestureGrid', () => {
  const render = (bound: string[]) => renderToStaticMarkup(<GestureGrid id="g" label="Gesture" gestures={['clockwise', 'counter_clockwise', 'pressed', 'released']} value="pressed" bound={gesture => bound.includes(gesture)} onChange={() => {}} />);

  it('offers one radio tile per gesture with the chosen one checked and focusable', () => {
    const markup = render([]);
    expect(markup).toContain('role="radiogroup"');
    expect(markup.match(/role="radio"/g)).toHaveLength(4);
    expect(markup).toMatch(/data-gesture="pressed" aria-checked="true" tabindex="0"/);
    expect(markup).toMatch(/data-gesture="clockwise" aria-checked="false" tabindex="-1"/);
    expect(markup).toContain('--gesture-columns:2');
  });

  it('marks gestures without an action as unused', () => {
    const markup = render(['clockwise']);
    expect(markup).toMatch(/data-gesture="clockwise"[^>]*class="gesture-tile bound"/);
    expect(markup).toMatch(/data-gesture="pressed"[^>]*class="gesture-tile checked unbound"/);
    expect(markup.match(/class="gesture-status">Unused</g)).toHaveLength(3);
    expect(markup.match(/class="gesture-status">In use</g)).toHaveLength(1);
  });
});

describe('keySlotUsed', () => {
  it('counts folders, the Back key and action keys with a binding as used', () => {
    expect(keySlotUsed(null)).toBe(false);
    expect(keySlotUsed(null, true)).toBe(true);
    expect(keySlotUsed({ type: 'folder', page_id: 'p' })).toBe(true);
    expect(keySlotUsed({ type: 'action', label: 'A', bindings: [] })).toBe(false);
    expect(keySlotUsed({ type: 'action', label: 'A', bindings: [{ gesture: 'pressed', action: { type: 'volume', delta: 5 } }] })).toBe(true);
  });
});
