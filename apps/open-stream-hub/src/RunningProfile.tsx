import type { ProfileSummary } from './contracts';
import { SelectField } from './controls';
import { isActiveRun } from './service-runner';
import type { ServiceRun } from './service-runner';

const mixed = '__mixed';

// The runs that hold a profile now: a session that has not stopped.
const liveRuns = (runs: Record<string, ServiceRun>) => Object.values(runs).filter(run => isActiveRun(run) && run.session !== undefined);

// What the selector shows: the one saved profile every running module holds; "mixed" when they differ, or one runs an
// unsaved draft; the default profile when nothing runs; nothing without either.
export function runningProfileValue(runs: Record<string, ServiceRun>, defaultId: string | null): string {
  const live = liveRuns(runs);
  if (live.length === 0) return defaultId ?? '';
  const ids = new Set(live.map(run => run.profileId ?? null));
  return ids.size === 1 && !ids.has(null) ? [...ids][0]! : mixed;
}

export interface RunningProfileActions {
  setDefaultProfile(profileId: string): Promise<unknown>;
  switchProfile(sessionId: string, profileId: string, why: string): Promise<boolean>;
}

// Makes `profileId` the profile of every module: it becomes the default that modules starting by themselves use, and
// every running module switches to it (the usual safe switch: an executing run pauses for confirmation when the profile
// adds anything). A module that is not running stays as it is. Returns the notice to show.
export async function applyRunningProfile(profileId: string, profileName: string, runs: Record<string, ServiceRun>, actions: RunningProfileActions): Promise<string> {
  await actions.setDefaultProfile(profileId);
  const live = liveRuns(runs);
  const targets = live.filter(run => run.profileId !== profileId);
  if (live.length === 0) return `Nothing is running yet. Modules start with ${profileName}.`;
  if (targets.length === 0) return `Every running module already runs ${profileName}.`;
  const switched = (await Promise.all(targets.map(run => actions.switchProfile(run.serviceId, profileId, 'chosen in the sidebar')))).filter(Boolean).length;
  const failed = targets.length - switched;
  const done = `${profileName} now runs on ${switched} ${switched === 1 ? 'module' : 'modules'}.`;
  return failed ? `${done} ${failed} could not be switched.` : done;
}

// The sidebar's profile choice, above the separator over the help links. It is linked to the header's profile picker:
// choosing a profile in either opens it in the editor and makes it the profile of every module, so `onChoose` does both.
export function RunningProfile({ profiles, runs, defaultId, enabled, busy, onChoose }: {
  profiles: ProfileSummary[]; runs: Record<string, ServiceRun>; defaultId: string | null; enabled: boolean; busy: boolean; onChoose(profileId: string): void;
}) {
  if (!enabled) return null;
  const value = runningProfileValue(runs, defaultId);
  return <div className="running-profile">
    <label htmlFor="running-profile">Running profile</label>
    <SelectField id="running-profile" value={value} disabled={busy || profiles.length === 0} onChange={event => {
      if (event.target.value && event.target.value !== mixed) onChoose(event.target.value);
    }}>
      {value === '' && <option value="">Choose a profile</option>}
      {value === mixed && <option value={mixed}>Different profiles</option>}
      {profiles.map(profile => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
    </SelectField>
  </div>;
}
