export type ServiceModule = 'm003k' | 'm131e' | 'm4315' | 'm005s' | 'm032le';
export type Gesture = 'clockwise' | 'counter_clockwise' | 'pressed' | 'released' | 'right' | 'left' | 'short_press' | 'long_press' | 'position_changed' | 'up' | 'down';
export type ReplayKind = 'knobs' | 'dial' | 'keys' | 'faders' | 'rollers';
export interface PositionValue { type: 'position'; raw: number; maximum: number }
export type Modifier = 'command' | 'control' | 'option' | 'shift';
export type BindingAction =
  | { type: 'volume'; delta: number }
  | { type: 'toggle_mute' }
  | { type: 'shortcut'; key: string; modifiers: Modifier[] }
  | { type: 'launch_app'; bundle_id: string }
  | { type: 'set_volume'; percent: number }
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
export interface Profile { schema_version: number; name: string; bindings: Binding[]; keypad?: { pages: KeypadPage[] } }
export interface StoredProfile { id: string; profile: Profile; modified_at_ms: number }
export interface ProfileSummary { id: string; name: string; binding_count: number; modified_at_ms: number }
export interface ValidationResult { valid: true }
export interface UnreadableProfile { id: string; error: string }
export interface ProfileListing { profiles: ProfileSummary[]; unreadable: UnreadableProfile[] }
// profile is null when storage holds only unreadable profiles; they are kept for recovery.
export interface AppBootstrap { profile: StoredProfile | null; profiles: ProfileSummary[]; unreadable: UnreadableProfile[]; storage_path: string; desktop: boolean }
export interface BootstrapError { message: string; storage_path: string | null }
export interface KeypadNavigation { from_page: string; to_page: string; path: string[]; depth: number }
export interface ReplayEvent { timestamp_us: number; module: ServiceModule; control: number; gesture: Gesture; value?: PositionValue; serial_number: string | null; action: Action | null; page_id?: string; navigation?: KeypadNavigation }
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
export interface StartServiceRequest { profile: Profile; path_hex: string; execute: boolean; software_mode: boolean }
export type ServiceState = 'starting' | 'running' | 'stopping' | 'exited' | 'failed';
export interface ServiceStatusPayload { service_id: string; state: ServiceState; execute: boolean; exit_code?: number | null; error?: string; stderr_tail?: string }
export interface ServiceRecordPayload { service_id: string; record: unknown }
