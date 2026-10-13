import type { Profile } from './contracts';
import type { ProfileSession } from './profile-session';
import { maxScreenBrightness, minScreenBrightness, screenBrightnessOf, setScreenBrightness } from './screen-brightness';
import type { BrightnessScreen } from './screen-brightness';
import { errorMessage } from './ui-model';

// What each screen's brightness does, shown under the slider.
export const screenBrightnessNotes: Record<BrightnessScreen, string> = {
  keys: 'The 15 key screens have no backlight setting, so OpenStreamHub draws each picture dimmer. Applies the next time this profile starts.',
  dial: 'The dial screen’s own backlight. Applies the next time this profile starts.',
};

// One screen's brightness for this profile, as a profile setting that travels with it.
export function ScreenBrightnessControl({ screen, label, profile, session, disabled }: { screen: BrightnessScreen; label: string; profile: Profile; session: ProfileSession; disabled: boolean }) {
  const value = screenBrightnessOf(profile, screen);
  const id = `screen-brightness-${screen}`;
  return <div className="screen-brightness">
    <label htmlFor={id}>{label}</label>
    <div className="screen-brightness-row">
      <input id={id} type="range" min={minScreenBrightness} max={maxScreenBrightness} step={5} value={value} disabled={disabled} aria-describedby={`${id}-note`}
        onChange={event => { try { const percent = Number(event.target.value); session.update(latest => setScreenBrightness(latest, screen, percent), `screen-brightness:${screen}`); } catch (reason) { session.setError(errorMessage(reason)); } }} />
      <span className="screen-brightness-value" aria-hidden="true">{value}%</span>
    </div>
    <p id={`${id}-note`} className="fine-print">{screenBrightnessNotes[screen]}</p>
  </div>;
}
