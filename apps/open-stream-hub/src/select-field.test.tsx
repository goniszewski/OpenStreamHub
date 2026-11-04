import { Fragment } from 'react';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { listPlacement, nextChoice, SelectField, selectChoices } from './controls';
import { choicesOf, shownBy } from './select-markup';

describe('SelectField', () => {
  const choices = selectChoices(<>
    <option value="a">Alpha</option>
    <Fragment><option value="b" disabled>Beta</option>{false}</Fragment>
    {['c', 'd'].map(value => <option key={value} value={value}>{`Option ${value}`}</option>)}
    <option>No value</option>
  </>);

  it('reads options from fragments, arrays and plain text, with a missing value taken from the label', () => {
    expect(choices).toEqual([
      { value: 'a', label: 'Alpha', disabled: false },
      { value: 'b', label: 'Beta', disabled: true },
      { value: 'c', label: 'Option c', disabled: false },
      { value: 'd', label: 'Option d', disabled: false },
      { value: 'No value', label: 'No value', disabled: false },
    ]);
  });
  it('steps over disabled choices and wraps around at both ends', () => {
    expect(nextChoice(choices, 0, 1)).toBe(2);
    expect(nextChoice(choices, 4, 1)).toBe(0);
    expect(nextChoice(choices, 2, -1)).toBe(0);
    expect(nextChoice(choices, 0, -1)).toBe(4);
    expect(nextChoice(choices, -1, 1)).toBe(0);
    expect(nextChoice(choices, -1, -1)).toBe(4);
    expect(nextChoice([{ value: 'x', label: 'X', disabled: true }], 0, 1)).toBe(0);
  });
  it('renders a closed combobox that shows the choice, with the list hidden but present', () => {
    const html = renderToStaticMarkup(<SelectField id="pick" value="c" aria-describedby="note" onChange={() => {}}>
      <option value="a">Alpha</option><option value="c">Option c</option>
    </SelectField>);
    expect(html).toContain('role="combobox"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain('aria-describedby="note"');
    expect(html).toMatch(/<ul id="pick-list"[^>]*hidden=""/);
    expect(shownBy(html, 'pick')).toBe('Option c');
    expect(choicesOf(html, 'pick')).toEqual([
      { value: 'a', label: 'Alpha', selected: false, disabled: false },
      { value: 'c', label: 'Option c', selected: true, disabled: false },
    ]);
  });
});

describe('listPlacement', () => {
  const trigger = (top: number) => ({ left: 20, right: 170, top, bottom: top + 36 });
  it('opens below the select when the list fits there, in window coordinates', () => {
    expect(listPlacement(trigger(100), 800, 200)).toEqual({ left: 20, width: 150, top: 142, maxHeight: 280 });
  });
  it('opens above the select near the bottom of the window, when there is more room above', () => {
    // The sidebar's profile select: low in a short window.
    expect(listPlacement(trigger(440), 525, 400)).toEqual({ left: 20, width: 150, bottom: 91, maxHeight: 280 });
  });
  it('stays below and shrinks when neither side has room for it and below has more', () => {
    expect(listPlacement(trigger(50), 300, 400)).toEqual({ left: 20, width: 150, top: 92, maxHeight: 200 });
  });
  it('never gives the list less than a few rows', () => {
    expect(listPlacement(trigger(10), 100, 400).maxHeight).toBe(80);
  });
});
