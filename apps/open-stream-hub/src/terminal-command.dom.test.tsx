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

describe('terminal_command fields in the DOM', () => {
  it('edits the app, the command and the optional folder, with inline issues', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'terminal_command', app: 'terminal', command: 'claude' }} seen={seen} />);
    const command = () => view!.$<HTMLInputElement>('#terminal-command');
    const directory = () => view!.$<HTMLInputElement>('#terminal-directory');
    expect(command().value).toBe('claude');
    expect(directory().value).toBe('');
    expect(view.container.querySelector('.field-error')).toBeNull();
    expect(view.$('#terminal-app .select-value').textContent).toBe('Terminal');

    view.click(view.$('#terminal-app'));
    view.click(view.$('#terminal-app-list [data-value="iterm2"]'));
    expect(seen.at(-1)).toEqual({ type: 'terminal_command', app: 'iterm2', command: 'claude' });
    expect(view.$('#terminal-app .select-value').textContent).toBe('iTerm2');

    type(command(), 'codex --model "o3"');
    expect(seen.at(-1)).toEqual({ type: 'terminal_command', app: 'iterm2', command: 'codex --model "o3"' });

    type(command(), '   ');
    expect(view.$('#terminal-command-error').textContent).toBe('Enter the command to run, such as claude.');
    expect(command().getAttribute('aria-invalid')).toBe('true');
    expect(command().getAttribute('aria-describedby')).toBe('terminal-command-error');
    type(command(), 'codex');
    expect(view.container.querySelector('#terminal-command-error')).toBeNull();

    type(directory(), 'code');
    expect(view.$('#terminal-directory-error').textContent).toBe('Use a full path starting with / or ~/.');
    type(directory(), '~/code/my app');
    expect(view.container.querySelector('#terminal-directory-error')).toBeNull();
    expect(seen.at(-1)).toEqual({ type: 'terminal_command', app: 'iterm2', command: 'codex', directory: '~/code/my app' });

    // An emptied folder is left out, never written as "".
    type(directory(), '');
    expect(seen.at(-1)).toEqual({ type: 'terminal_command', app: 'iterm2', command: 'codex' });
    expect('directory' in seen.at(-1)!).toBe(false);
  });
});
