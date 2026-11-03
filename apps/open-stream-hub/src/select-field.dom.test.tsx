// @vitest-environment jsdom
import { useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { SelectField } from './controls';
import { render } from './dom-test';
import type { Rendered } from './dom-test';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

function Harness({ initial = 'a', disabled = false, extra = false, seen }: { initial?: string; disabled?: boolean; extra?: boolean; seen?: string[] }) {
  const [value, setValue] = useState(initial);
  return <SelectField id="pick" value={value} disabled={disabled} onChange={event => { seen?.push(event.target.value); setValue(event.target.value); }}>
    <option value="a">Alpha</option><option value="b" disabled>Beta</option><option value="c">Charlie</option><option value="d">Delta</option>
    {extra && <option value="e">Echo</option>}
  </SelectField>;
}
const trigger = () => view!.$<HTMLButtonElement>('#pick');
const list = () => view!.$<HTMLUListElement>('#pick-list');
const shown = () => view!.$('.select-value').textContent;

describe('SelectField in the DOM', () => {
  it('opens on click, picks an option on click and closes', () => {
    const seen: string[] = [];
    view = render(<Harness seen={seen} />);
    expect(list().hidden).toBe(true);
    view.click(trigger());
    expect(list().hidden).toBe(false);
    expect(trigger().getAttribute('aria-expanded')).toBe('true');
    view.click(view.$('[data-value="c"]'));
    expect(seen).toEqual(['c']);
    expect(shown()).toBe('Charlie');
    expect(list().hidden).toBe(true);
  });
  it('steps with the arrow keys over a disabled option, and Enter picks the highlighted one', () => {
    const seen: string[] = [];
    view = render(<Harness seen={seen} />);
    view.key(trigger(), 'ArrowDown'); // opens, with the current choice highlighted
    expect(trigger().getAttribute('aria-activedescendant')).toBe('pick-option-0');
    view.key(trigger(), 'ArrowDown'); // Beta is disabled, so it lands on Charlie
    expect(trigger().getAttribute('aria-activedescendant')).toBe('pick-option-2');
    view.key(trigger(), 'Enter');
    expect(seen).toEqual(['c']);
    expect(list().hidden).toBe(true);
  });
  it('jumps with Home and End and wraps', () => {
    view = render(<Harness />);
    view.key(trigger(), 'Enter');
    view.key(trigger(), 'End');
    expect(trigger().getAttribute('aria-activedescendant')).toBe('pick-option-3');
    view.key(trigger(), 'ArrowDown');
    expect(trigger().getAttribute('aria-activedescendant')).toBe('pick-option-0');
    view.key(trigger(), 'Home');
    expect(trigger().getAttribute('aria-activedescendant')).toBe('pick-option-0');
  });
  it('picks by typing the start of a label', () => {
    const seen: string[] = [];
    view = render(<Harness seen={seen} />);
    view.key(trigger(), 'Enter');
    view.key(trigger(), 'd');
    view.key(trigger(), 'Enter');
    expect(seen).toEqual(['d']);
  });
  it('closes on Escape without picking and does not pick a disabled option', () => {
    const seen: string[] = [];
    view = render(<Harness seen={seen} />);
    view.key(trigger(), 'Enter');
    view.key(trigger(), 'ArrowDown');
    view.key(trigger(), 'Escape');
    expect(list().hidden).toBe(true);
    view.click(trigger());
    view.click(view.$('[data-value="b"]'));
    expect(seen).toEqual([]);
    expect(shown()).toBe('Alpha');
  });
  it('closes when it becomes disabled while open', () => {
    view = render(<Harness />);
    view.click(trigger());
    expect(list().hidden).toBe(false);
    view.rerender(<Harness disabled />);
    expect(list().hidden).toBe(true);
    expect(trigger().getAttribute('aria-expanded')).toBe('false');
  });
  it('drops a highlight that points past the end when the options shrink', () => {
    view = render(<Harness extra />);
    view.key(trigger(), 'Enter');
    view.key(trigger(), 'End');
    expect(trigger().getAttribute('aria-activedescendant')).toBe('pick-option-4');
    view.rerender(<Harness />);
    expect(trigger().getAttribute('aria-activedescendant')).toBeNull();
  });
});
