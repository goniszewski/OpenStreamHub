// @vitest-environment jsdom
import { act, useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { ActionFields } from './ActionFields';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { BindingAction } from './contracts';

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
});
