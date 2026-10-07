export type ServiceModule = 'm003k' | 'm131e' | 'm4315' | 'm005s' | 'm032le';
export type Gesture = 'clockwise' | 'counter_clockwise' | 'pressed' | 'released' | 'right' | 'left' | 'short_press' | 'long_press' | 'position_changed' | 'up' | 'down';
export type ReplayKind = 'knobs' | 'dial' | 'keys' | 'faders' | 'rollers';
export interface PositionValue { type: 'position'; raw: number; maximum: number }
export type Modifier = 'command' | 'control' | 'option' | 'shift';
export type MediaCommand = 'play_pause' | 'next_track' | 'previous_track' | 'stop';
export type SystemCommand = 'undo' | 'redo' | 'screenshot';
// The neighbouring saved profile a 'profile' action asks for. The OpenStreamHub session manager resolves it.
export type ProfileDirection = 'next' | 'previous';
// One step of a multi_action: any executable action except another multi_action.
export type StepAction =
  | { type: 'volume'; delta: number }
  | { type: 'toggle_mute' }
  // key: one ASCII letter, digit or space, or a named key (see namedKeys in action-model.ts).
  | { type: 'shortcut'; key: string; modifiers: Modifier[] }
  | { type: 'launch_app'; bundle_id: string }
  | { type: 'set_volume'; percent: number }
  | { type: 'open_url'; url: string }
  | { type: 'media'; command: MediaCommand }
  | { type: 'system'; command: SystemCommand }
  // press_enter may be omitted in JSON (defaults to false); the service always writes it.
  | { type: 'text'; text: string; press_enter?: boolean };
export type BindingAction =
  | StepAction
  | { type: 'multi_action'; actions: StepAction[] }
  // Never a multi_action step: the switch stops the running service, so later steps could not run.
  | { type: 'profile'; direction: ProfileDirection }
  | { type: 'volume_from_position' };
export type Action = Exclude<BindingAction, { type: 'volume_from_position' }>;
export interface Binding {
  module: ServiceModule;
  control: number;
  gesture: Gesture;
  serial_number?: string;
  action: BindingAction;
}
export interface KeypadBinding { gesture: 'pressed' | 'released'; serial_number?: string; action: Action }
export type KeypadSlot = { type: 'action'; label: string; bindings: KeypadBinding[] } | { type: 'folder'; page_id: string } | null;
export interface KeypadPage { id: string; name: string; slots: KeypadSlot[] }
// Dial function bindings (m131e only): short press cycles functions, so only these gestures are bound.
export interface DialBinding { gesture: 'right' | 'left' | 'long_press'; serial_number?: string; action: Action }
export interface DialFunction { name: string; bindings: DialBinding[] }
// dial_functions is edited in the Profiles page (DialFunctionsPanel) or imported; edit paths that do not touch it must carry it through untouched.
export interface Profile { schema_version: number; name: string; bindings: Binding[]; keypad?: { pages: KeypadPage[] }; dial_functions?: DialFunction[] }
export interface StoredProfile { id: string; profile: Profile; modified_at_ms: number }
export interface ProfileSummary { id: string; name: string; binding_count: number; modified_at_ms: number }
export interface ValidationResult { valid: true }
export interface UnreadableProfile { id: string; error: string }
export interface ProfileListing { profiles: ProfileSummary[]; unreadable: UnreadableProfile[] }
// profile is null when storage holds only unreadable profiles; they are kept for recovery.
export interface AppBootstrap { profile: StoredProfile | null; profiles: ProfileSummary[]; unreadable: UnreadableProfile[]; storage_path: string; desktop: boolean }
export interface BootstrapError { message: string; storage_path: string | null }
export interface KeypadNavigation { from_page: string; to_page: string; path: string[]; depth: number }
export interface DialFunctionSwitch { from_function: string; to_function: string; index: number; count: number }
// dial_function names the function that routed the event; dial_function_switch marks a short click that only switched functions (action is null then).
export interface ReplayEvent { timestamp_us: number; module: ServiceModule; control: number; gesture: Gesture; value?: PositionValue; serial_number: string | null; action: Action | null; page_id?: string; navigation?: KeypadNavigation; dial_function?: string; dial_function_switch?: DialFunctionSwitch }
export interface KeypadPreviewResult { path: string[]; depth: number; events: ReplayEvent[]; planned_action_count: number }
export interface ReplayResult {
  source: string;
  synthetic: true;
  status: 'completed';
  report_count: number;
  event_count: number;
  planned_action_count: number;
  events: ReplayEvent[];
  diagnostics: { timestamp_us: number; message: string }[];
}

// Live device feed (Tauri events and commands). Field names are snake_case on the wire.
export interface LiveInputEvent { timestamp_micros: number; control: number; gesture: Gesture; value?: PositionValue }
export interface DeviceInputPayload { feed_id: string; module: ServiceModule; serial_number: string | null; event: LiveInputEvent }
export type FeedState = 'running' | 'finished' | 'stopped' | 'failed';
export interface DeviceStatusPayload { feed_id: string; state: FeedState; source: 'demo' | 'live'; module: ServiceModule | null; error?: string }
export interface LiveDevice { path_hex: string; module: ServiceModule; serial_number: string | null }
export interface DeviceInventoryPayload { devices: LiveDevice[] }
export type FeedSource = { kind: 'demo'; demo: ReplayKind } | { kind: 'live'; path_hex: string };

// Supervised masterhub-service child process (Tauri commands and events, snake_case on the wire).
// profile_id: the saved profile the run starts from unchanged (omitted for an unsaved draft). A profile key's next/previous starts there.
export interface StartServiceRequest { profile: Profile; path_hex: string; execute: boolean; software_mode: boolean; profile_id?: string }
export type ServiceState = 'starting' | 'running' | 'stopping' | 'exited' | 'failed';
export interface ServiceStatusPayload { service_id: string; state: ServiceState; execute: boolean; exit_code?: number | null; error?: string; stderr_tail?: string }
export interface ServiceRecordPayload { service_id: string; record: unknown }

// Session manager (sleep/wake and reconnect for service runs). session_id is the first service_id and stays stable.
export type SessionPhase = 'running' | 'suspended' | 'waiting_for_device' | 'reconnecting' | 'needs_confirmation' | 'stopped';
// profile_switch: a focused-app rule replaced the profile; profile_action: a profile key (next/previous) did.
export type SessionReason = 'sleep' | 'device_removed' | 'service_exited' | 'user' | 'profile_switch' | 'profile_action';
export interface SessionStatePayload {
  session_id: string; service_id: string | null; phase: SessionPhase; reason: SessionReason | null; attempt: number; execute: boolean;
  module: ServiceModule; serial_number: string | null;
  // Bumped by every profile switch. resume_session must name the generation whose profile the user confirmed.
  profile_generation: number;
  // The profile of profile_generation: profile_id is the saved profile a switch put in place (null while the session
  // still runs the profile it was started with), profile_name its name. The runner resyncs its confirmation copy from them.
  profile_id: string | null; profile_name: string;
  detail?: string; next_attempt_in_ms?: number;
}

// Focused-app profile rules (stored in <app data>/focus-rules.json, never inside a profile). Opt-in: enabled is false by default.
export interface FocusRule { id: string; app_bundle_id: string; profile_id: string }
export interface FocusRules { enabled: boolean; rules: FocusRule[] }
// save_focus_rule argument: a null id adds a rule, an existing id edits it.
export interface FocusRuleInput { id: string | null; app_bundle_id: string; profile_id: string }
// focus://changed: rule_id and profile_id are set only when the rules are enabled and a rule matches an existing profile.
export interface FocusChangedPayload { bundle_id: string | null; rule_id: string | null; profile_id: string | null }
// switch_session_profile result: the saved profile the session now uses and its new profile generation.
export interface SwitchedProfile extends StoredProfile { profile_generation: number }
// session_profile: the profile a paused session holds for profile_generation, exactly as resume_session restarts it.
export interface HeldProfile { profile_id: string | null; profile_generation: number; profile: Profile }
// The last app other than OpenStreamHub that was frontmost. supported is false off macOS.
export interface FocusCurrentApp { supported: boolean; bundle_id: string | null }

// Experimental module display images (display_settings / set_display_settings; stored in
// <app data>/display-settings.json). Off by default; read once when a profile is started, so a change applies at the
// next start. dial_content is what the M131E dial shows while enabled.
export type DialContent = 'off' | 'function' | 'clock' | 'analog_clock';
export interface DisplaySettings { enabled: boolean; dial_content: DialContent }
