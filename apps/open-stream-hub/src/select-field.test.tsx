import { Fragment } from 'react';
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { nextChoice, SelectField, selectChoices } from './controls';
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
