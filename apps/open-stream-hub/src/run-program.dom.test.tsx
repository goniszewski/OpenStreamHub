// @vitest-environment jsdom
import { act, useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { ActionFields } from './ActionFields';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { BindingAction } from './contracts';
import { VariablesContext } from './variables-context';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

function Harness({ initial, seen }: { initial: BindingAction; seen: BindingAction[] }) {
  const [action, setAction] = useState<BindingAction | null>(initial);
  return <ActionFields action={action} fader={false} selectorKey="m4315:root:0:pressed:"
    onChange={next => { if (next) seen.push(next); setAction(next); }} />;
}

// Types into a controlled input the way the browser does: the native value setter, then an input event React hears.
function type(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

const button = (view: Rendered, text: string) =>
  [...view.container.querySelectorAll('button')].find(item => item.textContent === text || item.getAttribute('aria-label') === text)!;

describe('run_program fields in the DOM', () => {
  it('edits the path, each argument and the optional folder, with inline issues', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'run_program', program: '/usr/bin/say', args: ['Hello'] }} seen={seen} />);
    const path = () => view!.$<HTMLInputElement>('#program-path');
    expect(path().value).toBe('/usr/bin/say');
    expect(view.$<HTMLInputElement>('#program-arg-1').value).toBe('Hello');
    expect(view.container.querySelector('.field-error')).toBeNull();

    type(path(), 'say');
    expect(view.$('#program-path-error').textContent).toContain('starting with / or ~/');
    type(path(), '~/bin/lights');
    expect(view.container.querySelector('#program-path-error')).toBeNull();

    // An argument is passed exactly as typed, spaces and quotes included.
    type(view.$<HTMLInputElement>('#program-arg-1'), 'on air "now"');
    view.click(button(view, 'Add argument'));
    type(view.$<HTMLInputElement>('#program-arg-2'), '--fast');
    expect(seen.at(-1)).toEqual({ type: 'run_program', program: '~/bin/lights', args: ['on air "now"', '--fast'] });

    // Removing every argument omits the list.
    view.click(button(view, 'Remove argument 1'));
    view.click(button(view, 'Remove argument 1'));
    expect(seen.at(-1)).toEqual({ type: 'run_program', program: '~/bin/lights' });

    const directory = () => view!.$<HTMLInputElement>('#program-directory');
    type(directory(), 'relative');
    expect(view.$('#program-directory-error').textContent).toContain('/ or ~/');
    type(directory(), '~/code');
    expect(seen.at(-1)).toEqual({ type: 'run_program', program: '~/bin/lights', directory: '~/code' });
    // An emptied folder is omitted, never written as "".
    type(directory(), '');
    expect(seen.at(-1)).toEqual({ type: 'run_program', program: '~/bin/lights' });
  });

  it('turns filling in variables on and off, and checks each reference against the declared variables', () => {
    const seen: BindingAction[] = [];
    view = render(<VariablesContext.Provider value={[{ name: 'scene', value: 'intro' }]}>
      <Harness initial={{ type: 'run_program', program: '/a', args: ['--scene={{scene}}', '{{gone}}'] }} seen={seen} />
    </VariablesContext.Provider>);
    const toggle = () => view!.$<HTMLInputElement>('#program-variables');
    // Off: {{ is plain text, so nothing is flagged.
    expect(toggle().checked).toBe(false);
    expect(view.container.querySelector('.field-error')).toBeNull();
    expect(view.container.textContent).toContain('While off, {{ is passed as plain text.');
    view.click(toggle());
    expect(seen.at(-1)).toEqual({ type: 'run_program', program: '/a', args: ['--scene={{scene}}', '{{gone}}'], variables: true });
    expect(view.container.querySelector('#program-arg-1-error')).toBeNull();
    expect(view.$('#program-arg-2-error').textContent).toBe('Variable gone is not declared.');
    expect(view.container.textContent).toContain('Uses scene, gone.');
    type(view.$<HTMLInputElement>('#program-arg-2'), '{{ scene }}');
    expect(view.$('#program-arg-2-error').textContent).toContain('Each {{ must begin a variable');
    // Off again omits the flag rather than writing false.
    view.click(toggle());
    expect(seen.at(-1)).toEqual({ type: 'run_program', program: '/a', args: ['--scene={{scene}}', '{{ scene }}'] });
    expect(view.container.querySelector('.field-error')).toBeNull();
  });

  it('hints that an argument starting with a number or text variable fails when the value starts with -', () => {
    view = render(<VariablesContext.Provider value={[{ name: 'takes', value: 0 }, { name: 'live', value: false }, { name: 'scene', value: 'intro' }]}>
      <Harness initial={{ type: 'run_program', program: '/a', args: ['{{takes}}', '{{live}}', '--n={{takes}}', '{{scene}}'], variables: true }} seen={[]} />
    </VariablesContext.Provider>);
    expect(view.$('#program-arg-1-hint').textContent).toBe('When takes starts with - (a negative number), the press refuses to run the program. Write it as --option={{takes}} if the program takes one.');
    expect(view.$('#program-arg-1').getAttribute('aria-describedby')).toBe('program-arg-1-hint');
    // A bool never starts with -, and a written prefix keeps the value from leading.
    expect(view.container.querySelector('#program-arg-2-hint')).toBeNull();
    expect(view.container.querySelector('#program-arg-3-hint')).toBeNull();
    expect(view.$('#program-arg-4-hint').textContent).toContain('text such as --all');
    expect(view.container.querySelector('.field-error')).toBeNull();
  });
});
