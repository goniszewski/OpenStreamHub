import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { clampScreenBrightness, screenBrightnessOf, setScreenBrightness } from './screen-brightness';

const profile: Profile = { schema_version: 1, name: 'Desk', bindings: [] };

describe('screen brightness', () => {
  it('reads full brightness when nothing is stored', () => {
    expect(screenBrightnessOf(profile, 'keys')).toBe(100);
    expect(screenBrightnessOf({ ...profile, screen_brightness: { dial: 40 } }, 'dial')).toBe(40);
    expect(screenBrightnessOf({ ...profile, screen_brightness: { dial: 40 } }, 'keys')).toBe(100);
  });

  it('stores a dimmer screen and keeps the other screen untouched', () => {
    const dimmed = setScreenBrightness(setScreenBrightness(profile, 'keys', 60), 'dial', 30);
    expect(dimmed.screen_brightness).toEqual({ keys: 60, dial: 30 });
    expect(setScreenBrightness(dimmed, 'keys', 80).screen_brightness).toEqual({ keys: 80, dial: 30 });
  });

  it('stores full brightness as nothing and drops the setting when no screen is dimmed', () => {
    const dimmed = setScreenBrightness(setScreenBrightness(profile, 'keys', 60), 'dial', 30);
    expect(setScreenBrightness(dimmed, 'keys', 100).screen_brightness).toEqual({ dial: 30 });
    const restored = setScreenBrightness(setScreenBrightness(profile, 'keys', 60), 'keys', 100);
    expect('screen_brightness' in restored).toBe(false);
    expect(restored).toEqual(profile);
  });

  it('keeps a value inside the range the profile accepts', () => {
    expect(clampScreenBrightness(0)).toBe(10);
    expect(clampScreenBrightness(250)).toBe(100);
    expect(clampScreenBrightness(Number.NaN)).toBe(100);
    expect(setScreenBrightness(profile, 'dial', 3).screen_brightness).toEqual({ dial: 10 });
  });
});
