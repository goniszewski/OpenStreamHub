import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { variableUseCounts, variableUsers } from './variable-model';
import { VariablesPanel } from './VariableFields';

const profile = {
  schema_version: 5, name: 'p', variables: [{ name: 'live', value: false }],
  bindings: [
    { module: 'm003k', control: 1, gesture: 'pressed', action: { type: 'toggle_variable', name: 'live' } },
    { module: 'm003k', control: 2, gesture: 'pressed', action: { type: 'toggle_mute' } },
  ],
} as unknown as Profile;

describe('variableUsers', () => {
  it('lists the bindings that use a variable and none for an unused one', () => {
    expect(variableUsers(profile, 'live').map(entry => entry.control)).toEqual([1]);
    expect(variableUsers(profile, 'other')).toEqual([]);
  });
});

describe('VariablesPanel used by', () => {
  it('names each trigger that uses the variable as a button', () => {
    const html = renderToStaticMarkup(<VariablesPanel variables={profile.variables!} uses={variableUseCounts(profile)} users={name => variableUsers(profile, name)} onShow={() => {}} disabled={false} onChange={() => {}} onRename={() => {}} />);
    expect(html).toContain('Used 1 time, by:');
    expect(html).toContain('class="link-button"');
    expect(html).toContain('Knob 2');
  });
});
