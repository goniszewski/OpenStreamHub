import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { renameVariable, variableUseCounts, variableUsers } from './variable-model';
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

describe('run_program arguments with variables', () => {
  const program = (variables: boolean) => ({ type: 'run_program', program: '/a', args: ['--scene={{scene}}', '{{scene}}{{scenes}}', '{{ scene }}'], ...(variables ? { variables: true } : {}) });
  const withPrograms = {
    schema_version: 5, name: 'p', variables: [{ name: 'scene', value: 'intro' }, { name: 'scenes', value: 0 }],
    bindings: [
      { module: 'm003k', control: 1, gesture: 'pressed', action: program(true) },
      { module: 'm003k', control: 2, gesture: 'pressed', action: { type: 'multi_action', actions: [program(false)] } },
    ],
  } as unknown as Profile;
  it('count each {{name}} as a use only with variables on, and list the binding as a user', () => {
    // The malformed third argument names nothing (the service refuses it); the off copy in the multi_action counts none.
    expect(Object.fromEntries(variableUseCounts(withPrograms))).toEqual({ scene: 2, scenes: 1 });
    expect(variableUsers({ ...withPrograms, bindings: [{ ...withPrograms.bindings[0], action: { ...program(true), args: ['{{scene}}'] } }] } as unknown as Profile, 'scene').map(entry => entry.control)).toEqual([1]);
  });
  it('rename the references with the declaration, with variables on or off, and leave a longer name alone', () => {
    const renamed = renameVariable(withPrograms, 'scene', 'shot');
    expect(renamed.variables?.map(variable => variable.name)).toEqual(['shot', 'scenes']);
    const args = ['--scene={{shot}}', '{{shot}}{{scenes}}', '{{ scene }}'];
    expect(renamed.bindings[0].action).toEqual({ ...program(true), args });
    // Renamed while off too, so turning variables back on still names a declared variable.
    expect(renamed.bindings[1].action).toEqual({ type: 'multi_action', actions: [{ ...program(false), args }] });
  });
});

describe('VariablesPanel used by', () => {
  it('names each trigger that uses the variable as a button', () => {
    const html = renderToStaticMarkup(<VariablesPanel variables={profile.variables!} uses={variableUseCounts(profile)} users={name => variableUsers(profile, name)} onShow={() => {}} disabled={false} onChange={() => {}} onRename={() => {}} />);
    expect(html).toContain('Used by 1 trigger:');
    expect(html).toContain('class="link-button"');
    expect(html).toContain('Knob 2');
  });
});
