import type { ServiceRun } from './service-runner';

// What the editor does when a profile key switches a run to another saved profile: open that profile, so the module image
// matches the device. Only a key switch counts (a focused-app rule and the profile picker already have their own paths),
// only a profile the editor does not already show, and never over unsaved changes, which stay untouched.
export interface FollowStep {
  // The saved profile to open in the editor, if any.
  load: string | null;
  // The switched profile's name when the editor kept its unsaved changes instead, for a note.
  held: string | null;
  // Generation seen per run key; pass it back in on the next call.
  seen: Record<string, number>;
}

export function followRunProfile(runs: Record<string, ServiceRun>, seen: Record<string, number> | null, editor: { profileId: string | null; unsaved: boolean }): FollowStep {
  const next: Record<string, number> = {};
  let load: string | null = null;
  let held: string | null = null;
  for (const [key, run] of Object.entries(runs)) {
    const generation = run.profileGeneration ?? 0;
    next[key] = generation;
    // The first look only records where each run is, so opening the app on a run that was switched earlier changes nothing.
    if (seen === null || generation <= (seen[key] ?? 0)) continue;
    if (run.profileSwitchedBy !== 'key' || !run.profileSwitched || !run.profileId || run.profileId === editor.profileId) continue;
    if (editor.unsaved) held = run.profileName;
    else load = run.profileId;
  }
  return { load, held, seen: next };
}
