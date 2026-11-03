// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import type { BindingEntry } from './binding-model';
import type { Profile } from './contracts';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import { VariableSelect, VariablesPanel } from './VariableFields';
import { variableUseCounts, variableUsers } from './variable-model';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; document.body.innerHTML = ''; });

const profile = {
  schema_version: 5, name: 'p', variables: [{ name: 'live', value: false }],
  bindings: [
    { module: 'm003k', control: 1, gesture: 'pressed', action: { type: 'toggle_variable', name: 'live' } },
    { module: 'm003k', control: 2, gesture: 'pressed', action: { type: 'toggle_variable', name: 'live' } },
  ],
} as unknown as Profile;

describe('the Variables panel in the DOM', () => {
  it('opens the binding of the trigger that is pressed', () => {
    const shown: BindingEntry[] = [];
    view = render(<VariablesPanel variables={profile.variables!} uses={variableUseCounts(profile)} users={name => variableUsers(profile, name)} onShow={entry => shown.push(entry)} disabled={false} onChange={() => {}} onRename={() => {}} />);
    const buttons = view.$$('.variable-users .link-button');
    expect(buttons).toHaveLength(2);
    view.click(buttons[1]);
    expect(shown.map(entry => entry.control)).toEqual([2]);
  });
});

describe('the empty variable select', () => {
  it('opens and focuses the Variables section from its link', () => {
    document.body.insertAdjacentHTML('beforeend', '<details class="inspector-variables"><summary>Variables</summary><button id="variable-add">Add variable</button></details>');
    const details = document.querySelector<HTMLDetailsElement>('.inspector-variables')!;
    view = render(<VariableSelect id="v" label="Variable" value="" kinds={['bool']} onChange={() => {}} />);
    expect(view.$('[role="note"]').textContent).toContain('No on / off variable is declared yet.');
    expect(details.open).toBe(false);
    view.click(view.$('.link-button'));
    expect(details.open).toBe(true);
    expect(document.activeElement?.id).toBe('variable-add');
  });
});
