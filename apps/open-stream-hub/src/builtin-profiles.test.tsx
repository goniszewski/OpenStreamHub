import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { BuiltInProfilesPanel } from './BuiltInProfilesPanel';
import { builtInOf, createHiddenBuiltInsStore, hiddenBuiltInsStorageKey, profileChoices } from './builtin-profiles';
import type { ProfileSummary } from './contracts';

const saved = (id: string, name: string): ProfileSummary => ({ id, name, binding_count: 0, modified_at_ms: 0, dry_run_only: false });
const memory = (initial?: string) => { const data = new Map<string, string>(initial === undefined ? [] : [[hiddenBuiltInsStorageKey, initial]]); return { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); } }; };

describe('built-in profiles in the pickers', () => {
  it('lists the built-in ones that are not saved yet after the saved profiles', () => {
    const choices = profileChoices([saved('a', 'Everyday'), saved('m', 'Mac basics')], new Set());
    expect(choices.map(item => [item.value, item.builtIn])).toEqual([['a', false], ['m', false], ['builtin:streaming', true], ['builtin:editing', true]]);
  });
  it('leaves a hidden one out, saved or not, except the profile that is open or runs', () => {
    const list = [saved('a', 'Everyday'), saved('m', 'Mac basics')];
    expect(profileChoices(list, new Set(['mac-basics', 'streaming', 'editing'])).map(item => item.value)).toEqual(['a']);
    expect(profileChoices(list, new Set(['mac-basics', 'streaming', 'editing']), new Set(['m'])).map(item => item.value)).toEqual(['a', 'm']);
  });
  it('finds the template a built-in value stands for', () => {
    expect(builtInOf('builtin:streaming')?.name).toBe('Streaming (OBS)');
    expect(builtInOf('builtin:nothing')).toBeUndefined();
    expect(builtInOf('a')).toBeUndefined();
  });
  it('remembers what is hidden, and shows everything when nothing is stored or the store is damaged', () => {
    const storage = memory();
    const store = createHiddenBuiltInsStore(storage);
    expect([...store.getSnapshot()]).toEqual([]);
    store.setShown('editing', false);
    expect([...createHiddenBuiltInsStore(storage).getSnapshot()]).toEqual(['editing']);
    store.setShown('editing', true);
    expect([...createHiddenBuiltInsStore(storage).getSnapshot()]).toEqual([]);
    expect([...createHiddenBuiltInsStore(memory('not json')).getSnapshot()]).toEqual([]);
  });
  it('shows a switch for every built-in profile in Settings', () => {
    const markup = renderToStaticMarkup(<BuiltInProfilesPanel saved={[saved('m', 'Mac basics')]} hidden={new Set(['editing'])} />);
    expect(markup).toContain('Show Mac basics in the profile pickers');
    expect(markup).toContain('You have saved it.');
    expect(markup.match(/type="checkbox"/g)).toHaveLength(3);
    expect(markup.match(/checked=""/g)).toHaveLength(2);
  });
});
