// @vitest-environment jsdom
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from './dom-test';
import { tooltipDelayMs, tooltipPlacement, useControlTooltip } from './ControlTooltip';

const tip = { title: 'Knob 1', rows: [{ gesture: null, name: 'Toggle output mute', description: 'Mutes or unmutes the Mac output.' }] };
function Harness() {
  const tooltip = useControlTooltip(() => tip);
  return <div><button type="button" {...tooltip.bind(0)}>Knob</button>{tooltip.node}</div>;
}
// React derives pointerenter/leave from pointerover/out.
const fire = (target: Element, type: string) => act(() => { target.dispatchEvent(new Event(type === 'pointerenter' ? 'pointerover' : type === 'pointerleave' ? 'pointerout' : type, { bubbles: true })); });

describe('control tooltip', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); document.body.innerHTML = ''; });

  it('shows the name and description after a hover delay, and hides on leave', () => {
    const view = render(<Harness />);
    const button = view.$('button');
    fire(button, 'pointerenter');
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    act(() => { vi.advanceTimersByTime(tooltipDelayMs); });
    const card = document.querySelector('[role="tooltip"]')!;
    expect(card.textContent).toContain('Toggle output mute');
    expect(card.textContent).toContain('Mutes or unmutes the Mac output.');
    expect(button.getAttribute('aria-describedby')).toBe(card.id);
    fire(button, 'pointerleave');
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
  });
  it('hides on Escape and on press', () => {
    const view = render(<Harness />);
    const button = view.$('button');
    fire(button, 'pointerenter');
    act(() => { vi.advanceTimersByTime(tooltipDelayMs); });
    act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); });
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    fire(button, 'pointerenter');
    act(() => { vi.advanceTimersByTime(tooltipDelayMs); });
    fire(button, 'pointerdown');
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
  });
});

describe('tooltipPlacement', () => {
  const viewport = { width: 1000, height: 700 };
  it('sits under the control', () => expect(tooltipPlacement({ centre: 500, top: 100, bottom: 140 }, viewport)).toEqual({ left: 500, top: 148 }));
  it('flips above near the bottom edge', () => expect(tooltipPlacement({ centre: 500, top: 600, bottom: 640 }, viewport)).toEqual({ left: 500, bottom: 108 }));
  it('stays inside the window sideways', () => {
    expect(tooltipPlacement({ centre: 10, top: 100, bottom: 140 }, viewport).left).toBe(148);
    expect(tooltipPlacement({ centre: 995, top: 100, bottom: 140 }, viewport).left).toBe(852);
  });
});
