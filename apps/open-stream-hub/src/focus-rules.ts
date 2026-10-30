import type { FocusChangedPayload, FocusCurrentApp, FocusRule, FocusRuleInput, FocusRules, RunningApp } from './contracts';
import type { Listen } from './device-feed';
import type { Invoke } from './profile-session';
import type { ServiceRun } from './service-runner';
import { errorMessage } from './ui-model';

// Focused-app profile rules. Rust stores and matches them (src-tauri/src/focus.rs); this store edits them and,
// when focus://changed carries a matched profile, asks the service runner to switch every live session to it.

export const focusChangedEvent = 'focus://changed';
export const maxFocusRules = 64;
export const maxBundleIdLength = 255;
// Rapid app switching (Cmd-Tab through several apps) settles before anything is switched; only the latest event counts.
export const focusDebounceMs = 400;

// Mirrors focus::normalize_bundle_id for inline feedback; Rust validates again on save.
export function bundleIdIssue(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return 'Enter an app bundle ID, for example com.apple.Safari.';
  if (trimmed.length > maxBundleIdLength) return `An app bundle ID has at most ${maxBundleIdLength} characters.`;
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(trimmed)) return "Use only letters, digits, '.', '-' and '_', starting with a letter or digit.";
  return null;
}

// Live sessions (managed runs that have not stopped) that are not already on, or switching to, `profileId`.
export function sessionsToSwitch(runs: Record<string, ServiceRun>, profileId: string): string[] {
  return Object.entries(runs)
    .filter(([, run]) => run.session !== undefined && run.session.phase !== 'stopped' && run.profileId !== profileId)
    .map(([key]) => key);
}

// What the header says about automatic switching, or null when it is off (or there is no desktop to watch focus).
// `matched` is the saved profile the app in front is switched to, when a rule for it exists and its profile too.
export interface FocusStatus { text: string; matched: string | null }
export function focusStatus(state: Pick<FocusRulesState, 'desktop' | 'enabled' | 'rules' | 'focusedApp'>, profiles: { id: string; name: string }[]): FocusStatus | null {
  if (!state.desktop || !state.enabled) return null;
  const count = `${state.rules.length} rule${state.rules.length === 1 ? '' : 's'}`;
  const app = state.focusedApp;
  const rule = app ? state.rules.find(item => item.app_bundle_id.toLowerCase() === app.toLowerCase() && profiles.some(profile => profile.id === item.profile_id)) : undefined;
  const matched = rule ? profiles.find(profile => profile.id === rule.profile_id)!.name : null;
  if (matched) return { text: `Switching profiles automatically (${count}). ${app} is in front, so the profile is “${matched}”.`, matched };
  return { text: `Switching profiles automatically (${count}). ${app ? `${app} is in front and has no rule.` : 'Waiting for another app to come to the front.'}`, matched: null };
}

// The rules that switch to one profile, in the order the rules are checked.
export const rulesForProfile = (rules: FocusRule[], profileId: string | null): FocusRule[] => profileId === null ? [] : rules.filter(rule => rule.profile_id === profileId);

// The apps the picker offers: running apps that have no rule yet (an app has one rule; the first match wins).
export function appChoices(apps: RunningApp[], rules: FocusRule[]): RunningApp[] {
  const ruled = new Set(rules.map(rule => rule.app_bundle_id.toLowerCase()));
  return apps.filter(app => !ruled.has(app.bundle_id.toLowerCase()));
}

// The apps running now (list_running_apps), well-formed entries only; none when the list cannot be read. The action
// editor's app pickers share it with the store.
export async function listRunningApps(invoke: Invoke): Promise<RunningApp[]> {
  try {
    const apps = await invoke<RunningApp[]>('list_running_apps');
    return Array.isArray(apps) ? apps.filter(app => typeof app?.bundle_id === 'string' && typeof app?.name === 'string') : [];
  } catch { return []; }
}

// An app by its name when it is running, else by its bundle id.
export const appName = (bundleId: string, apps: RunningApp[]): string =>
  apps.find(app => app.bundle_id.toLowerCase() === bundleId.toLowerCase())?.name ?? bundleId;

export interface FocusRulesState {
  desktop: boolean; loaded: boolean; busy: boolean; error: string | null;
  enabled: boolean; rules: FocusRules['rules'];
  // The last other app reported by focus://changed, for display.
  focusedApp: string | null;
}
export const idleFocusRulesState: FocusRulesState = { desktop: false, loaded: false, busy: false, error: null, enabled: false, rules: [], focusedApp: null };

interface SwitchTarget { getState(): { runs: Record<string, ServiceRun> }; switchProfile(sessionId: string, profileId: string, why: string): Promise<boolean> }
export interface FocusTimers { set(callback: () => void, ms: number): unknown; clear(handle: unknown): void }
const defaultTimers: FocusTimers = { set: (callback, ms) => setTimeout(callback, ms), clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>) };

export class FocusRulesStore {
  private state: FocusRulesState;
  private readonly listeners = new Set<() => void>();
  private pendingSwitch: unknown = null;
  // Rule edits, deletes and disables still waiting for their reply. A switch matched against the old rules could fire
  // before the reply lands, so the debounced switch is skipped while any is in flight and cancelled again on reply.
  private rulesChanging = 0;

  constructor(private readonly invoke: Invoke, private readonly listen: Listen, desktop: boolean, private readonly runner?: SwitchTarget,
    private readonly timers: FocusTimers = defaultTimers) {
    this.state = { ...idleFocusRulesState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<FocusRulesState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Follows focus://changed. Returns the unsubscribe function.
  connect(): () => void {
    if (!this.state.desktop) return () => {};
    let active = true;
    let unlisten: (() => void) | undefined;
    this.listen(focusChangedEvent, event => this.onFocus(event.payload as FocusChangedPayload)).then(
      remove => { if (active) unlisten = remove; else remove(); },
      reason => { if (active) this.set({ error: errorMessage(reason) }); });
    return () => { active = false; unlisten?.(); this.cancelSwitch(); };
  }

  // Each focus change replaces the pending one; switching happens focusDebounceMs after the last change, for its match.
  // Editing, deleting or disabling rules cancels a pending switch, which was matched against the old rules, and no
  // switch fires until that change's reply arrived.
  onFocus(payload: FocusChangedPayload) {
    if (!payload || typeof payload !== 'object') return;
    if (typeof payload.bundle_id === 'string' && payload.bundle_id !== this.state.focusedApp) this.set({ focusedApp: payload.bundle_id });
    this.cancelSwitch();
    const profileId = payload.profile_id;
    if (typeof profileId !== 'string' || !this.runner) return;
    const runner = this.runner;
    const why = `focused ${payload.bundle_id ?? 'app'}`;
    const ruleId = payload.rule_id;
    this.pendingSwitch = this.timers.set(() => {
      this.pendingSwitch = null;
      if (this.rulesChanging > 0) return;
      // Rules edited while the switch waited win: once loaded here, the rules must still be on and still map to it.
      const { loaded, enabled, rules } = this.state;
      if (loaded && !(enabled && rules.some(rule => rule.id === ruleId && rule.profile_id === profileId))) return;
      for (const key of sessionsToSwitch(runner.getState().runs, profileId)) void runner.switchProfile(key, profileId, why);
    }, focusDebounceMs);
  }

  private cancelSwitch() {
    if (this.pendingSwitch !== null) this.timers.clear(this.pendingSwitch);
    this.pendingSwitch = null;
  }

  private async run(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!this.state.desktop) return false;
    this.set({ busy: true });
    try {
      const rules = await this.invoke<FocusRules>(command, args);
      this.set({ busy: false, loaded: true, error: null, enabled: rules.enabled, rules: rules.rules });
      return true;
    } catch (reason) {
      this.set({ busy: false, error: errorMessage(reason) });
      return false;
    }
  }

  load() { return this.run('list_focus_rules'); }
  // Cancels a pending switch and holds later ones until the change's reply arrived; then cancels whatever was
  // scheduled meanwhile, since it was matched against the rules before the change.
  private async change(command: string, args: Record<string, unknown>): Promise<boolean> {
    this.cancelSwitch();
    this.rulesChanging += 1;
    try { return await this.run(command, args); }
    finally { this.rulesChanging -= 1; this.cancelSwitch(); }
  }

  save(rule: FocusRuleInput) { return this.change('save_focus_rule', { rule: { ...rule, app_bundle_id: rule.app_bundle_id.trim() } }); }
  remove(id: string) { return this.change('delete_focus_rule', { id }); }
  setEnabled(enabled: boolean) { return enabled ? this.run('set_focus_rules_enabled', { enabled }) : this.change('set_focus_rules_enabled', { enabled }); }

  // The apps running now, for the picker; none (without an error) when the list cannot be read, since typing the
  // bundle ID still works.
  async runningApps(): Promise<RunningApp[]> {
    return this.state.desktop ? listRunningApps(this.invoke) : [];
  }

  // The last app other than OpenStreamHub that was frontmost, or null with an explanation in `error`.
  async currentApp(): Promise<string | null> {
    if (!this.state.desktop) return null;
    try {
      const app = await this.invoke<FocusCurrentApp>('focus_current_app');
      if (!app.supported) { this.set({ error: 'Detecting the focused app needs macOS. Type the bundle ID instead.' }); return null; }
      if (!app.bundle_id) { this.set({ error: 'No other app has been in front yet. Switch to the app once, then come back.' }); return null; }
      this.set({ error: null });
      return app.bundle_id;
    } catch (reason) {
      this.set({ error: errorMessage(reason) });
      return null;
    }
  }

  dismissError() { this.set({ error: null }); }
}
