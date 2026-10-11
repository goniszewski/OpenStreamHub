import type { DialDisplay, DialDisplayContent, DialSource, Profile } from './contracts';

// What the M131E dial screen shows while this profile runs and the app drives
// the display (Profile.dial_display). The Rust authority is the DialDisplay
// type in crates/cooltek-sdk-service/src/dial.rs and the app's
// src-tauri/src/display.rs, which turns it into --dial-content; keep the
// value list and the fallback in step.

// Mirrors DialDisplay::MAX_IMAGE_BYTES.
export const maxDialImageBytes = 64 * 1024;

export const dialDisplayOptions: readonly { value: DialDisplayContent; label: string }[] = [
  { value: 'function', label: 'Function name (default)' },
  { value: 'clock', label: 'Digital clock (HH:MM)' },
  { value: 'analog_clock', label: 'Analog clock' },
  { value: 'cpu', label: 'CPU usage (%, macOS only)' },
  { value: 'image', label: 'Custom image' },
  { value: 'off', label: 'Nothing (keys only)' },
];

// The masterhub-service --dial-content spelling of each value.
export const dialDisplayArgs: Record<DialDisplayContent, string> = {
  off: 'off', function: 'function', clock: 'clock', analog_clock: 'analog-clock', cpu: 'cpu', image: 'image',
};

const bindsDial = (profile: Profile) =>
  profile.dial_functions !== undefined || profile.bindings.some(binding => binding.module === 'm131e');

// What the dial actually shows: the profile's choice, or the default function
// name — which becomes the calm analog clock when the profile has nothing on
// the dial (no functions, no binding), because there would be no name to show.
// Mirrors display::dial_content in src-tauri/src/display.rs.
export function effectiveDialDisplay(profile: Profile): DialDisplayContent {
  const content = profile.dial_display?.content ?? 'function';
  if (content === 'function' && !bindsDial(profile)) return 'analog_clock';
  return content;
}

// `profile` with the dial screen set to `content`. Choosing anything but the
// custom image clears a stored picture, so a saved profile never carries an
// image it does not show.
export function setDialDisplayContent(profile: Profile, content: DialDisplayContent): Profile {
  const display: DialDisplay = { content };
  return { ...profile, dial_display: display };
}

// `profile` with the custom dial picture set (content already image).
export function setDialImage(profile: Profile, image: string, source?: DialSource): Profile {
  return { ...profile, dial_display: source ? { content: 'image', image, source } : { content: 'image', image } };
}
