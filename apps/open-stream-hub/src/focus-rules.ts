import type { FocusChangedPayload, FocusCurrentApp, FocusRuleInput, FocusRules } from './contracts';
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
  // Editing, deleting or disabling rules cancels a pending switch, which was matched against the old rules.
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
  save(rule: FocusRuleInput) { this.cancelSwitch(); return this.run('save_focus_rule', { rule: { ...rule, app_bundle_id: rule.app_bundle_id.trim() } }); }
  remove(id: string) { this.cancelSwitch(); return this.run('delete_focus_rule', { id }); }
  setEnabled(enabled: boolean) { if (!enabled) this.cancelSwitch(); return this.run('set_focus_rules_enabled', { enabled }); }

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
