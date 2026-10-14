import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ProfileSummary } from './contracts';
import { RunningProfile, applyRunningProfile, runningProfileValue } from './RunningProfile';
import type { ServiceRun } from './service-runner';

const run = (id: string, profileId: string | null, phase: 'running' | 'stopped' = 'running'): ServiceRun => ({
  serviceId: id, state: 'running', execute: false, entries: [], counters: { actions: 0, failures: 0, ignored: 0 } as never, startedAt: 0, profileName: 'x', profileId, snapshot: '', draftUnsaved: false,
  session: { serviceId: `p-${id}`, phase, reason: null, attempt: 0, module: 'm4315', serialNumber: id },
});
const runs = (...items: ServiceRun[]) => Object.fromEntries(items.map(item => [item.serviceId, item]));
const profiles = [{ id: 'a', name: 'Everyday' }, { id: 'b', name: 'Studio' }] as ProfileSummary[];

describe('the sidebar running-profile choice', () => {
  it('shows the one profile every running module holds, else the default, else nothing', () => {
    expect(runningProfileValue(runs(run('k', 'a'), run('d', 'a')), 'b')).toBe('a');
    expect(runningProfileValue(runs(), 'b')).toBe('b');
    expect(runningProfileValue(runs(), null)).toBe('');
    // Stopped sessions do not count.
    expect(runningProfileValue(runs(run('k', 'a', 'stopped')), 'b')).toBe('b');
  });

  it('says so when modules run different profiles or an unsaved draft', () => {
    expect(runningProfileValue(runs(run('k', 'a'), run('d', 'b')), 'a')).toBe('__mixed');
    expect(runningProfileValue(runs(run('k', null)), 'a')).toBe('__mixed');
    const html = renderToStaticMarkup(<RunningProfile profiles={profiles} runs={runs(run('k', 'a'), run('d', 'b'))} defaultId={null} enabled busy={false} onChoose={() => {}} />);
    expect(html).toContain('Different profiles');
    expect(html).toContain('Running profile');
  });

  it('lists the saved profiles with the running one selected, and stays out of the browser preview', () => {
    const html = renderToStaticMarkup(<RunningProfile profiles={profiles} runs={runs(run('k', 'b'))} defaultId="a" enabled busy={false} onChoose={() => {}} />);
    expect(html).toMatch(/<option value="b" selected="">Studio<\/option>/);
    expect(html).toContain('Everyday');
    expect(renderToStaticMarkup(<RunningProfile profiles={profiles} runs={runs()} defaultId={null} enabled={false} busy={false} onChoose={() => {}} />)).toBe('');
    expect(renderToStaticMarkup(<RunningProfile profiles={profiles} runs={runs()} defaultId="a" enabled busy onChoose={() => {}} />)).toContain('disabled=""');
  });

  it('makes the profile the default and switches every running module that holds another one', async () => {
    const switched: string[] = [];
    let defaultId = '';
    const actions = {
      setDefaultProfile: async (id: string) => { defaultId = id; },
      switchProfile: async (session: string, id: string) => { switched.push(`${session}>${id}`); return session !== 'bad'; },
    };
    const note = await applyRunningProfile('b', 'Studio', runs(run('k', 'a'), run('d', 'b'), run('bad', 'a'), run('s', 'a', 'stopped')), actions);
    expect(defaultId).toBe('b');
    // The module already on it and the stopped one are left alone.
    expect(switched.sort()).toEqual(['bad>b', 'k>b']);
    expect(note).toBe('Studio now runs on 1 module. 1 could not be switched.');
    expect(await applyRunningProfile('b', 'Studio', runs(run('k', 'a'), run('d', 'a')), { ...actions, switchProfile: async () => true })).toBe('Studio now runs on 2 modules.');
    expect(await applyRunningProfile('b', 'Studio', runs(run('d', 'b')), actions)).toBe('Every running module already runs Studio.');
    expect(await applyRunningProfile('b', 'Studio', runs(), actions)).toBe('Nothing is running yet. Modules start with Studio.');
  });
});
