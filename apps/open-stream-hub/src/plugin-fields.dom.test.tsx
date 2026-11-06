// @vitest-environment jsdom
import { act, useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { ActionFields } from './ActionFields';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { BindingAction } from './contracts';
import { PluginsContext } from './plugins-context';
import type { PluginListing } from './plugins-model';
import { VariablesContext } from './variables-context';
import { lights, listing } from './plugins-test-data';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

function Harness({ initial, seen, plugins = listing }: { initial: BindingAction; seen: BindingAction[]; plugins?: PluginListing | null }) {
  const [action, setAction] = useState<BindingAction | null>(initial);
  return <VariablesContext.Provider value={[{ name: 'room', value: 'desk' }]}>
    <PluginsContext.Provider value={{ listing: plugins, error: null, refresh: () => {} }}>
      <ActionFields action={action} fader={false} selectorKey="m4315:root:0:pressed:" onChange={next => { if (next) seen.push(next); setAction(next); }} />
    </PluginsContext.Provider>
  </VariablesContext.Provider>;
}

function type(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

const choose = (view: Rendered, select: string, value: string) => {
  view.click(view.$(`#${select}`));
  view.click(view.$(`#${select}-list [data-value="${value}"]`));
};

describe('plugin action fields in the DOM', () => {
  it('takes the first installed plugin for a fresh pick and generates the form from its manifest', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: '', min_version: '1.0.0', action: '' }} seen={seen} />);
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'toggle' });
    expect(view.container.querySelector('#plugin-param-entity')).toBeNull();

    choose(view, 'plugin-action', 'set');
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { level: 50, mode: 'warm' } });
    // The required light is missing until typed.
    expect(view.$('#plugin-param-entity-error').textContent).toBe('Light is required.');
    type(view.$<HTMLInputElement>('#plugin-param-entity'), 'light.{{room}}');
    expect(view.container.querySelector('#plugin-param-entity-error')).toBeNull();
    expect(seen.at(-1)).toMatchObject({ params: { entity: 'light.{{room}}' }, variables: ['entity'] });
    type(view.$<HTMLInputElement>('#plugin-param-level'), '140');
    expect(view.$('#plugin-param-level-error').textContent).toContain('between 0 and 100');
    view.click(view.$('#plugin-param-fade'));
    expect(seen.at(-1)).toMatchObject({ params: { fade: true } });
    // A secret is masked.
    expect(view.$<HTMLInputElement>('#plugin-param-token').type).toBe('password');
  });

  it('keeps a key whose plugin is missing and says why it will not run', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.gone', min_version: '1.0.0', action: 'x' }} seen={seen} />);
    expect(view.$('#plugin-problem').textContent).toContain('com.example.gone is not installed');
    expect(view.$('#plugin-id').textContent).toContain('com.example.gone (not installed)');
    expect(seen).toEqual([]);
  });

  it('says so when no plugin is installed', () => {
    view = render(<Harness initial={{ type: 'plugin', plugin: '', min_version: '1.0.0', action: '' }} seen={[]}
      plugins={{ folder: '/p', plugins: [] }} />);
    expect(view.container.textContent).toContain('No plugins are installed.');
    expect(view.container.textContent).toContain('Plugins folder: /p');
    expect(lights.actions.length).toBe(2);
  });
});
