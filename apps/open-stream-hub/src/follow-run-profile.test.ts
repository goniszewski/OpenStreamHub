import { describe, expect, it } from 'vitest';
import { followRunProfile } from './follow-run-profile';
import type { ServiceRun } from './service-runner';

const run = (extra: Partial<ServiceRun>): ServiceRun => ({ profileName: 'A', profileId: 'a', profileGeneration: 0, ...extra } as ServiceRun);
const switched = (id: string, generation = 1, by: ServiceRun['profileSwitchedBy'] = 'key') => run({ profileId: id, profileName: id.toUpperCase(), profileGeneration: generation, profileSwitched: true, profileSwitchedBy: by });
const clean = { profileId: 'a', unsaved: false };

describe('followRunProfile', () => {
  it('only records the runs on the first look', () => {
    expect(followRunProfile({ x: switched('b') }, null, clean)).toEqual({ load: null, held: null, seen: { x: 1 } });
  });
  it('opens the profile a key switched the run to', () => {
    expect(followRunProfile({ x: switched('b') }, { x: 0 }, clean)).toEqual({ load: 'b', held: null, seen: { x: 1 } });
  });
  it('treats a run it has not seen yet as generation 0', () => {
    expect(followRunProfile({ x: switched('b') }, {}, clean).load).toBe('b');
  });
  it('does nothing twice for the same generation', () => {
    expect(followRunProfile({ x: switched('b') }, { x: 1 }, clean).load).toBeNull();
  });
  it('keeps unsaved changes and reports the switched profile instead', () => {
    expect(followRunProfile({ x: switched('b') }, { x: 0 }, { profileId: 'a', unsaved: true })).toEqual({ load: null, held: 'B', seen: { x: 1 } });
  });
  it('does nothing when the editor already shows that profile', () => {
    expect(followRunProfile({ x: switched('b') }, { x: 0 }, { profileId: 'b', unsaved: true })).toEqual({ load: null, held: null, seen: { x: 1 } });
  });
  it('ignores focused-app rule switches, runs that were not switched, and a switch to an unknown profile', () => {
    expect(followRunProfile({ x: switched('b', 1, 'rule') }, { x: 0 }, clean).load).toBeNull();
    expect(followRunProfile({ x: { ...switched('b'), profileSwitchedBy: undefined } }, { x: 0 }, clean).load).toBeNull();
    expect(followRunProfile({ x: run({ profileGeneration: 1, profileSwitchedBy: 'key', profileSwitched: false }) }, { x: 0 }, clean).load).toBeNull();
    expect(followRunProfile({ x: run({ profileGeneration: 1, profileSwitchedBy: 'key', profileSwitched: true, profileId: null }) }, { x: 0 }, clean).load).toBeNull();
  });
  it('follows a draft that has no saved id by opening the switched profile when nothing is unsaved', () => {
    expect(followRunProfile({ x: switched('b') }, { x: 0 }, { profileId: null, unsaved: false }).load).toBe('b');
  });
});
