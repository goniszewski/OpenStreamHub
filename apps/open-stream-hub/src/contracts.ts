export type ServiceModule = 'm003k' | 'm131e' | 'm4315' | 'm005s' | 'm032le';
export type Gesture = 'clockwise' | 'counter_clockwise' | 'pressed' | 'released' | 'right' | 'left' | 'short_press' | 'long_press' | 'position_changed' | 'up' | 'down';
export type ReplayKind = 'knobs' | 'dial' | 'keys' | 'faders' | 'rollers';
export interface PositionValue { type: 'position'; raw: number; maximum: number }
export type Modifier = 'command' | 'control' | 'option' | 'shift';
// A hardware media key a 'shortcut' can send (MEDIA_KEYS in profile.rs). No virtual key code, so it
// never combines with modifiers and cannot be a macro key.
export type MediaKey = 'play_pause' | 'next_track' | 'previous_track';
// One recorded keystroke of a macro: the key held down and released, the modifiers held while it is
// down, the wait before it and how long it stays down (MacroStep in profile.rs).
export interface MacroStep { key: string; modifiers: Modifier[]; delay_before_ms: number; hold_ms: number }
export type MediaCommand = 'play_pause' | 'next_track' | 'previous_track' | 'stop';
export type SystemCommand = 'undo' | 'redo' | 'screenshot';
// The neighbouring saved profile a 'profile' action asks for. The OpenStreamHub session manager resolves it.
export type ProfileDirection = 'next' | 'previous';
// End sound of a 'timer' action: a closed set of macOS system sounds (TimerSound in profile.rs). Absent means glass.
export type TimerSound = 'basso' | 'blow' | 'bottle' | 'frog' | 'funk' | 'glass' | 'hero' | 'morse' | 'ping' | 'pop'
  | 'purr' | 'sosumi' | 'submarine' | 'tink' | 'none';
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
  | { type: 'text'; text: string; press_enter?: boolean }
  // Switches the OBS program scene over the local OBS WebSocket. The endpoint and password are app settings, never
  // part of a profile.
  | { type: 'obs_scene'; scene: string }
  // Start, stop or toggle the OBS virtual camera, or the replay buffer (also: save it), over the same local OBS
  // WebSocket. No names are involved; the op is a closed set (ObsOutputOp, ObsReplayBufferOp in profile.rs).
  | { type: 'obs_virtual_camera'; op: ObsOutputOp }
  | { type: 'obs_replay_buffer'; op: ObsReplayBufferOp }
  // Going live or ending a broadcast (obs_stream), and the recording including pause and resume (obs_record).
  | { type: 'obs_stream'; op: ObsOutputOp; confirm?: ObsConfirm }
  | { type: 'obs_record'; op: ObsRecordOp }
  // Studio mode, its transition, a source filter and a source's visibility in a scene. The names are exact OBS names
  // (the same rules as obs_scene.scene).
  | { type: 'obs_studio_mode'; op: ObsSwitchOp }
  | { type: 'obs_transition' }
  // Saves a PNG of the program scene into ~/Pictures (a fixed place; a profile cannot name a path).
  | { type: 'obs_screenshot' }
  | { type: 'obs_filter'; source: string; filter: string; op: ObsSwitchOp }
  | { type: 'obs_source_visibility'; scene: string; source: string; op: ObsSwitchOp }
  // Switch the OBS scene collection or OBS profile by exact name. OBS reloads, which can interrupt a live output.
  | { type: 'obs_scene_collection'; name: string }
  | { type: 'obs_profile'; name: string }
  // Set an OBS input's volume to a position on OBS's own slider (0 is silent, 100 is unity gain, 0 dB; the scale is OBS's logarithmic taper, not linear). The input is an exact OBS name.
  | { type: 'obs_set_input_volume'; input: string; percent: number }
  // Mute an input, step its volume by dB (held between -100 and 0 dB), or play/pause/restart/stop a media source.
  | { type: 'obs_input_mute'; input: string; op: ObsMuteOp }
  | { type: 'obs_adjust_input_volume'; input: string; step_db: number }
  | { type: 'obs_media_source'; input: string; op: ObsMediaOp };
export type ObsSwitchOp = 'toggle' | 'on' | 'off';
// With press_twice a key must be pressed twice within three seconds; only a key's own action, not a multi-action step.
export type ObsConfirm = 'press_twice';
export type ObsMuteOp = 'toggle' | 'mute' | 'unmute';
export type ObsMediaOp = 'play_pause' | 'play_stop' | 'play_restart' | 'stop';
export type ObsOutputOp = 'toggle' | 'start' | 'stop';
export type ObsReplayBufferOp = ObsOutputOp | 'save';
export type ObsRecordOp = ObsOutputOp | 'pause' | 'resume' | 'toggle_pause';
// The state a conditional tests (Condition in profile.rs): one key with a value, or all/any of several. The first four read an OBS output and
// need the OBS connection; the last two read the default output of this Mac.
export type Condition = LeafCondition | { all: LeafCondition[] } | { any: LeafCondition[] };
// `all` (AND) and `any` (OR) combine 2 to 8 plain conditions; they do not nest or mix.
export type LeafCondition =
  | { obs_streaming: boolean }
  | { obs_recording: boolean }
  | { obs_virtual_camera: boolean }
  | { obs_replay_buffer: boolean }
  | { output_muted: boolean }
  | { output_volume_at_least: number };
export type BindingAction =
  | StepAction
  | { type: 'multi_action'; actions: StepAction[] }
  // Tests one condition when pressed, then runs the `then` steps (1-8) or the `else` steps (0-8, omitted when empty).
  // Steps follow the multi_action rules; never a step itself.
  | { type: 'conditional'; when: Condition; then: StepAction[]; else?: StepAction[] }
  // A recorded sequence of keystrokes with waits and holds (bounded: 1–64 steps, ≤30 s in total).
  // Never a multi_action step: a macro is already a timed sequence.
  | { type: 'macro'; steps: MacroStep[] }
  // Never a multi_action step: the switch stops the running service, so later steps could not run.
  | { type: 'profile'; direction: ProfileDirection }
  // A countdown in the running service; pressing the same binding again cancels it. Never a multi_action step.
  | { type: 'timer'; seconds: number; label?: string; sound?: TimerSound }
  | { type: 'volume_from_position' }
  | { type: 'mic_input_volume_from_position' }
  // Fader-only: each position change sets the named OBS input's volume (obs_set_input_volume with the fader's percentage).
  | { type: 'obs_input_volume_from_position'; input: string };
export type FaderTemplate = Extract<BindingAction, { type: 'volume_from_position' | 'mic_input_volume_from_position' | 'obs_input_volume_from_position' }>;
export type Action = Exclude<BindingAction, FaderTemplate>;
export interface Binding {
  module: ServiceModule;
  control: number;
  gesture: Gesture;
  serial_number?: string;
  action: BindingAction;
}
export interface KeypadBinding { gesture: 'pressed' | 'released'; serial_number?: string; action: Action }
export type KeypadSlot = { type: 'action'; label: string; bindings: KeypadBinding[] } | { type: 'folder'; page_id: string } | null;
export interface KeyArtworkFrame { jpeg_base64: string; delay_ms: number }
// The icon-and-label design a key's artwork was prepared from, stored so the studio reopens
// with it (absent for uploaded images). Pure metadata: nothing drawn reads it. icon_id is a
// catalog id or `app:<bundle id>`, absent or null for text only; text_colour, icon_colour and
// background are hex as typed (`#rgb`/`rgb`/`#rrggbb`/`rrggbb`).
export interface KeyArtworkDesignSpec {
  icon_id?: string | null; text: string; text_size: number; icon_size?: number; align: 'top' | 'middle' | 'bottom';
  text_colour: string; icon_colour: string; background: string;
}
/** loop_count: zero is infinite; positive values are total plays. */
export interface KeyArtwork { frames: KeyArtworkFrame[]; loop_count: number; design?: KeyArtworkDesignSpec }
export interface KeypadPage { id: string; name: string; slots: KeypadSlot[]; artwork?: Record<string, KeyArtwork> }
// Dial function bindings (m131e only): short press cycles functions, so only these gestures are bound.
export interface DialBinding { gesture: 'right' | 'left' | 'long_press'; serial_number?: string; action: Action }
export interface DialFunction { name: string; bindings: DialBinding[] }
// What the M131E dial screen shows while the profile runs and the app drives the display. The values match the
// masterhub-service --dial-content spellings; 'image' shows the picture in DialDisplay.image. Absent means the
// default: the active function's name, or the analog clock for a profile with nothing on the dial.
export type DialDisplayContent = 'off' | 'function' | 'clock' | 'analog_clock' | 'cpu' | 'image';
// The picture the custom face was made from and how it was fitted, so the look can be changed after a reopen.
export interface DialSource { image: string; fit: 'contain' | 'cover' | 'stretch'; quarter_turns: number; background: [number, number, number] }
// A GIF prepared for the dial: 2 to 30 frames of 240×240 JPEG, each shown at least as long as it takes to send: 66, 100 or 150 ms by frame size. The first equals
// DialDisplay.image. loop_count 0 loops forever; a positive count is the number of plays, then the last frame holds.
export interface DialAnimationFrame { jpeg_base64: string; delay_ms: number }
export interface DialAnimation { frames: DialAnimationFrame[]; loop_count: number }
export interface DialDisplay { content: DialDisplayContent; image?: string; source?: DialSource; animation?: DialAnimation }
// dial_functions and dial_display are edited in the dial editor of the inspector or imported; edit paths that do not
// touch them must carry them through untouched.
// How bright the module screens are while the profile runs and the app drives them, in percent (10 to 100); a screen left
// out is at full brightness. Mirrors ScreenBrightness in crates/cooltek-sdk-service/src/profile.rs.
export interface ScreenBrightness { keys?: number; dial?: number }
// Profile file schemas this version reads (1 flat bindings; 2 keypad pages; 3 the same layout; 4 current, the one it writes, adds the conditional action). Mirrors
// Profile::OLDEST_SCHEMA and CURRENT_SCHEMA in crates/cooltek-sdk-service/src/profile.rs; a profile from a newer
// version is refused with a message saying so.
export const oldestSchema = 1;
export const currentSchema = 4;
// The keypad is a page tree (schema 2 and later) rather than flat bindings (schema 1).
export const usesKeypadPages = (profile: Pick<Profile, 'schema_version'>): boolean => profile.schema_version >= 2;
export interface Profile { schema_version: number; name: string; bindings: Binding[]; keypad?: { pages: KeypadPage[] }; dial_functions?: DialFunction[]; dial_display?: DialDisplay; screen_brightness?: ScreenBrightness }
// content_hash: the backend's hash of the profile without its name; allow_imported_profile_execution names the one the
// confirmation showed.
export interface StoredProfile { id: string; profile: Profile; modified_at_ms: number; content_hash?: string }
// One earlier version of a profile kept by saving (manual or automatic), named by the moment it was saved. Restoring is a
// load plus an ordinary save, so it goes through validation, conflict protection and the imported-content flags.
// name and binding_count are null for a kept file that cannot be read as a profile.
export interface ProfileVersion { modified_at_ms: number; name: string | null; binding_count: number | null }
// dry_run_only: an imported .mhz profile that may run only as a dry run until the user allows it to execute actions
// (allow_imported_profile_execution). Stored in <app data>/imported-profiles.json, never inside the profile.
export interface ProfileSummary { id: string; name: string; binding_count: number; modified_at_ms: number; dry_run_only: boolean }
export interface ValidationResult { valid: true }

// Official MasterHUB .mhz import (import_mhz_profile; the raw file bytes are the request body). Each profile in the
// file becomes a new saved, dry-run-only profile. The report never holds a file path or a device serial: detail is a
// redacted value (a basename, scheme://host, or "<N chars>").
export type ImportOutcome = 'imported' | 'imported_lossy' | 'unbound_unsupported' | 'unbound_dangerous';
export type ImportSection = 'keypad' | 'knobs' | 'dial' | 'rollers' | 'faders';
export interface ImportReportRow { section: ImportSection; path: string; official_action: string; outcome: ImportOutcome; reason: string; detail?: string; high_risk: boolean }
export interface ImportCounts { imported: number; imported_lossy: number; unbound_unsupported: number; unbound_dangerous: number }
export interface ImportCategoryCounts extends ImportCounts { category: string }
export interface ImportReport {
  profile_name: string; source_name: string; totals: ImportCounts; categories: ImportCategoryCounts[]; actions: ImportCategoryCounts[];
  high_risk: number; warnings: string[]; rows: ImportReportRow[];
}
export interface ImportedProfile { stored: StoredProfile; report: ImportReport }
export interface MhzImportResult { profiles: ImportedProfile[]; source_sha256: string }
export interface UnreadableProfile { id: string; error: string }
// import_flags_error: the imported profile list is missing or damaged. Every profile is then dry run only until the user
// chooses "Mark all profiles as needing review" (mark_all_profiles_for_review).
export interface ProfileListing { profiles: ProfileSummary[]; unreadable: UnreadableProfile[]; import_flags_error?: string | null }
// profile is null when storage holds only unreadable profiles; they are kept for recovery.
export interface AppBootstrap { profile: StoredProfile | null; profiles: ProfileSummary[]; unreadable: UnreadableProfile[]; import_flags_error?: string | null; storage_path: string; desktop: boolean }
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
// Top-left cell of a module on the 6 × 4 base; negative when the module starts beyond the top or left edge.
export interface BaseCell { col: number; row: number }
export interface LiveDevice { path_hex: string; module: ServiceModule; serial_number: string | null; base_cell?: BaseCell; base_turn?: number; port_chain?: string }
export interface DeviceInventoryPayload { devices: LiveDevice[] }
export type FeedSource = { kind: 'demo'; demo: ReplayKind } | { kind: 'live'; path_hex: string };

// Supervised masterhub-service child process (Tauri commands and events, snake_case on the wire).
// profile_id: the saved profile the run starts from unchanged (omitted for an unsaved draft). A profile key's next/previous starts there.
// source_profile_id: the saved profile the draft was opened from, also after edits; the backend refuses Execute when it is an
// imported dry-run-only profile.
export interface StartServiceRequest { profile: Profile; path_hex: string; execute: boolean; software_mode: boolean; profile_id?: string; source_profile_id?: string }
export type ServiceState = 'starting' | 'running' | 'stopping' | 'exited' | 'failed';
export interface ServiceStatusPayload { service_id: string; state: ServiceState; execute: boolean; exit_code?: number | null; error?: string; stderr_tail?: string }
export interface ServiceRecordPayload { service_id: string; record: unknown }

// Session manager (sleep/wake and reconnect for service runs). session_id is the first service_id and stays stable.
export type SessionPhase = 'running' | 'suspended' | 'waiting_for_device' | 'reconnecting' | 'needs_confirmation' | 'stopped';
// profile_switch: a focused-app rule replaced the profile; profile_action: a profile key (next/previous) did.
// profile_saved: the user saved the profile the session runs and it swapped to the saved content in place.
// start_refused: the service exited with an error before it reported service_start; the session stops without retrying.
// module_control: an automatic session ended because Module control is no longer App (or its file cannot be read).
// run_mode: the Live/Test mode was switched (or the execution guard's answer for a profile changed), so the session
// restarts with or without actions.
export type SessionReason = 'sleep' | 'device_removed' | 'service_exited' | 'start_refused' | 'user' | 'profile_switch' | 'profile_action' | 'profile_saved' | 'module_control' | 'run_mode' | 'demo';
// Set for the whole life of a session the session manager started on its own (Module control App): a dry run on the
// 15-key module or the dial when the app opened, when the module was connected, or when Module control was switched to
// App with the module connected. Absent for a user's Start.
export type SessionAutoStart = 'app_opened' | 'module_connected' | 'module_control_set';
export interface SessionStatePayload {
  session_id: string; service_id: string | null; phase: SessionPhase; reason: SessionReason | null; attempt: number; execute: boolean;
  module: ServiceModule; serial_number: string | null;
  // Bumped by every profile switch. resume_session must name the generation whose profile the user confirmed.
  profile_generation: number;
  // The profile of profile_generation: profile_id is the saved profile a switch put in place (null while the session
  // still runs the profile it was started with), profile_name its name. The runner resyncs its confirmation copy from them.
  profile_id: string | null; profile_name: string;
  // What made the latest profile switch, also when the running process took it in place (then `reason` stays null).
  profile_switch_reason?: SessionReason;
  detail?: string; next_attempt_in_ms?: number; auto_start?: SessionAutoStart;
  // Why this session runs as a dry run although the run mode is Live (the execution guard's refusal).
  auto_dry_run_reason?: string;
}

// Focused-app profile rules (stored in <app data>/focus-rules.json, never inside a profile). Opt-in: enabled is false by default.
export interface FocusRule { id: string; app_bundle_id: string; profile_id: string }
export interface FocusRules { enabled: boolean; rules: FocusRule[] }
// list_running_apps: an app with a Dock icon that is running now, by bundle id and display name.
export interface RunningApp { bundle_id: string; name: string }
// save_focus_rule argument: a null id adds a rule, an existing id edits it.
export interface FocusRuleInput { id: string | null; app_bundle_id: string; profile_id: string }
// focus://changed: rule_id and profile_id are set only when the rules are enabled and a rule matches an existing profile.
export interface FocusChangedPayload { bundle_id: string | null; rule_id: string | null; profile_id: string | null }
// switch_session_profile result: the saved profile the session now uses and its new profile generation.
export interface SwitchedProfile extends StoredProfile { profile_generation: number }
// session_profile: the profile a paused session holds for profile_generation, exactly as resume_session restarts it.
export interface HeldProfile { profile_id: string | null; profile_generation: number; profile: Profile }
// apply_saved_profile: one session the save applied to. A session with an error keeps running its previous profile;
// paused marks an executing session that will wait in needs_confirmation for the saved content.
export interface AppliedSession { session_id: string; profile_generation: number; paused: boolean; error?: string }
// apply_saved_profile result: the saved profile and every session it applies to.
export interface AppliedSavedProfile extends StoredProfile { sessions: AppliedSession[] }
// The last app other than OpenStreamHub that was frontmost. supported is false off macOS.
export interface FocusCurrentApp { supported: boolean; bundle_id: string | null }

// Module control (display_settings / set_display_settings; stored in <app data>/display-settings.json). 'app' (the
// default) runs the 15-key module and the dial in software mode with their displays in sync; 'built_in_animation'
// leaves them in the module's own mode. Read once when a profile is started, so a change applies at the next start.
// What the dial shows under 'app' is the running profile's dial_display setting, not a global one.
export type ModuleControl = 'app' | 'built_in_animation';
// run_mode: one mode for the whole app, shown by the header badge. 'live' (the default for a fresh install): a press
// runs its action (macOS only, behind the confirmations). 'test': presses are logged only, nothing happens on the Mac.
// Switching it applies to every running module; a profile imported from .mhz stays Test until it is allowed to execute.
// live_offered: whether the one-time what-will-run confirmation has been answered. False only on a fresh install, where
// automatic sessions stay dry (auto_dry_run_reason says so) until the app confirms the mode; files from before the
// field read as true, so existing users are never asked again.
// held_from: set only while the editing hold ("Switch to Test mode while editing") has Test stored on the user's behalf,
// naming the Live to restore when the editing ends — or at the next launch, when the app quit or crashed mid-edit.
export type RunMode = 'live' | 'test';
// press_feedback: whether a key on the 15-key module is drawn dimmed while it is held down (on by default).
export interface DisplaySettings { module_control: ModuleControl; run_mode: RunMode; live_offered: boolean; held_from?: 'live'; press_feedback: boolean }
// default_profile / set_default_profile: the saved profile every automatic start uses first, or null for none.
export interface DefaultProfile { profile_id: string | null }
// OBS WebSocket setting (src-tauri/src/obs.rs, <app data>/obs-settings.json). host is 127.0.0.1, ::1 or localhost.
// The password is never stored or returned: set_obs_password({ password }) stores it in the macOS Keychain (and holds it in memory) (null
// forgets it) and every reply says only whether one is set.
export type ObsHost = '127.0.0.1' | '::1' | 'localhost';
export interface ObsSettings { enabled: boolean; host: ObsHost; port: number }
export interface ObsSettingsView extends ObsSettings { password_set: boolean }
// The obs_status reply: what "Test connection" found, with one line on what to do.
export type ObsConnectionState = 'off' | 'connected' | 'not_running' | 'websocket_off' | 'wrong_port' | 'password_needed' | 'wrong_password' | 'failed';
export interface ObsConnectionView { state: ObsConnectionState; message: string }
// The OBS listings (obs_catalog, obs_source_filters, obs_scene_sources): what the editor pickers
// offer. Names are exact, in OBS's own order (the store sorts them for display only).
export interface ObsCatalogNames { scenes: string[]; sources: string[]; scene_collections: string[]; profiles: string[] }
// One name list an OBS editor field can offer: loading, the names, or why OBS could not be asked.
export type ObsNameList = { status: 'loading' } | { status: 'ready'; names: string[] } | { status: 'failed'; error: string }

// Macro recording (src-tauri/src/recorder.rs) and its permission preflights. One recording runs at
// a time, in the editor that started it. Recording needs Input Monitoring (listen_events); replaying
// recorded keys needs Accessibility (post_events). Both are checked, never requested, here.
export type PermissionTarget = 'post_events' | 'listen_events';
export interface InputPermissions { post_events: boolean; listen_events: boolean }
// One recorded keyboard event. key is the OpenStreamHub key name (or literal character) of the
// virtual key code, or null for a key no macro can hold (the editor drops those and says so).
export interface RecordedKey {
  key: string | null; key_code: number; down: boolean;
  shift: boolean; control: boolean; option: boolean; command: boolean;
  at_ms: number;
}
export interface RecordingView { events: RecordedKey[]; elapsed_ms: number; capped: boolean }
