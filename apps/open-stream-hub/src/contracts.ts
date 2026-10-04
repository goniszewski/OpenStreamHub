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
export interface Profile { schema_version: number; name: string; bindings: Binding[] }
export interface StoredProfile { id: string; profile: Profile; modified_at_ms: number }
export interface ProfileSummary { id: string; name: string; binding_count: number; modified_at_ms: number }
export interface ValidationResult { valid: true }
export interface AppBootstrap { profile: StoredProfile; profiles: ProfileSummary[]; storage_path: string; desktop: boolean }
export interface ReplayEvent { timestamp_us: number; module: ServiceModule; control: number; gesture: Gesture; value?: PositionValue; serial_number: string | null; action: Action | null }
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
