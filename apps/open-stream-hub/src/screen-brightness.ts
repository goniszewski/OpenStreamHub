import type { Profile, ScreenBrightness } from './contracts';

// How bright the module screens are while a profile runs and the app drives the display (Profile.screen_brightness).
// Mirrors ScreenBrightness in crates/cooltek-sdk-service/src/profile.rs: 10 to 100 percent, a screen left out is at
// full brightness. The dial has a backlight; the 15 keys have none, so the app draws their pictures dimmer.

export type BrightnessScreen = keyof ScreenBrightness;

export const minScreenBrightness = 10;
export const maxScreenBrightness = 100;

export const clampScreenBrightness = (value: number): number =>
  Math.min(maxScreenBrightness, Math.max(minScreenBrightness, Math.round(Number.isFinite(value) ? value : maxScreenBrightness)));

// The brightness shown for a screen: its stored value, or full.
export const screenBrightnessOf = (profile: Profile, screen: BrightnessScreen): number =>
  profile.screen_brightness?.[screen] ?? maxScreenBrightness;

// `profile` with one screen set to `value`. Full brightness is the default, so it is stored as nothing, and the whole
// setting is dropped when no screen is left dimmed: a profile that never touched it serializes unchanged.
export function setScreenBrightness(profile: Profile, screen: BrightnessScreen, value: number): Profile {
  const next: ScreenBrightness = { ...profile.screen_brightness };
  const percent = clampScreenBrightness(value);
  if (percent === maxScreenBrightness) delete next[screen];
  else next[screen] = percent;
  const { screen_brightness: _dropped, ...rest } = profile;
  return Object.keys(next).length === 0 ? rest : { ...rest, screen_brightness: next };
}
