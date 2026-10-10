import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Grid2X2, Lightbulb, RotateCcw, Star, X } from 'lucide-react';
import { SelectField } from './controls';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import {
  addColourFavourite, appliedStatus, applyDelayMs, applyLighting, clampBacklight, defaultBacklight, defaultColour,
  lightingBlockReason, lightingDeviceLabel, lightingScopeLabels, lightingTargets, readColourFavourites,
  removeColourFavourite, resetLighting, resetStatus, writeColourFavourites,
} from './lighting-model';
import type { LightingScope } from './lighting-model';
import { errorMessage } from './ui-model';
import type { Invoke } from './profile-session';
import type { ServiceRunner } from './service-runner';

const noopSubscribe = () => () => {};
const idleState = () => idleFeedState;

// The Lighting page (task 09, phase 1): the encoder dial's backlight and one
// solid colour, applied to the hardware as the controls move. Everything that
// decides — targets, gating, favourites, request shaping — lives in
// lighting-model; this component only holds the controls and debounces.
export function LightingPage({ desktop, invoke, feed, runner, storage = null }: {
  desktop: boolean; invoke?: Invoke; feed?: DeviceFeed; runner?: ServiceRunner;
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null;
}) {
  const devices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState).devices;
  const [scope, setScope] = useState<LightingScope>('module');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [backlight, setBacklight] = useState(defaultBacklight);
  const [colour, setColour] = useState(defaultColour);
  const [favourites, setFavourites] = useState<string[]>(() => readColourFavourites(storage));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  // The picked module, falling back to the first dial and then the first
  // module, so a replug or a scope flip never leaves the picker empty-handed.
  const selected = devices.find(device => device.path_hex === selectedPath)
    ?? devices.find(device => device.module === 'm131e') ?? devices[0];
  const targets = lightingTargets(devices, scope, selected?.path_hex ?? null);
  const runOwned = targets.some(device => runner?.isActivePath(device.path_hex, device) ?? false);
  const reason = lightingBlockReason({ desktop, runOwnedTargets: runOwned, scope, selected, devices });
  // The debounced apply reads the latest values and gate from this ref, not
  // from a render closure; a new change replaces the pending one.
  const latest = useRef({ invoke, reason, targets, backlight, colour });
  latest.current = { invoke, reason, targets, backlight, colour };
  const timer = useRef<number | null>(null);
  const inFlight = useRef(false);

  useEffect(() => () => { if (timer.current !== null) window.clearTimeout(timer.current); }, []);

  function scheduleApply() {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => { timer.current = null; void applyNow(); }, applyDelayMs);
  }

  async function applyNow() {
    const current = latest.current;
    if (current.reason !== null || !current.invoke || current.targets.length === 0 || inFlight.current) return;
    inFlight.current = true;
    setError(null);
    try {
      const written = await applyLighting(current.invoke, {
        targets: current.targets.map(device => device.path_hex), backlight: current.backlight, colour: current.colour,
      });
      setStatus(appliedStatus(written));
    } catch (reason) { setError(errorMessage(reason)); }
    inFlight.current = false;
  }

  async function reset() {
    const current = latest.current;
    if (current.reason !== null || !current.invoke || current.targets.length === 0 || busy) return;
    setBusy(true);
    setError(null);
    try {
      const written = await resetLighting(current.invoke, current.targets.map(device => device.path_hex));
      setStatus(resetStatus(written));
    } catch (reason) { setError(errorMessage(reason)); }
    setBusy(false);
  }

  function setBacklightValue(value: number) {
    setBacklight(clampBacklight(value));
    scheduleApply();
  }

  function setColourValue(value: string) {
    setColour(value);
    scheduleApply();
  }

  function saveFavourite() {
    const next = addColourFavourite(favourites, colour);
    setFavourites(next);
    writeColourFavourites(storage, next);
  }

  function dropFavourite(hex: string) {
    const next = removeColourFavourite(favourites, hex);
    setFavourites(next);
    writeColourFavourites(storage, next);
  }

  const controlsLocked = reason !== null;

  return <section className="page-content lighting-page">
    <div className="page-title"><div><h1>Lighting</h1><p>The encoder dial’s backlight and colour, applied to the hardware as you adjust them.</p></div></div>
    <div className="panel lighting-panel">
      <div className="theme-picker lighting-scope" role="group" aria-label="Lighting scope">
        {(Object.keys(lightingScopeLabels) as LightingScope[]).map(value => <button key={value} type="button"
          aria-pressed={scope === value} className={scope === value ? 'active' : ''} onClick={() => setScope(value)}>
          {value === 'module' ? <Lightbulb size={15} /> : <Grid2X2 size={15} />}{lightingScopeLabels[value]}
        </button>)}
      </div>
      {scope === 'module' && <div className="lighting-module">
        <label htmlFor="lighting-module">Module</label>
        <SelectField id="lighting-module" value={selected?.path_hex ?? ''} disabled={devices.length === 0}
          onChange={event => setSelectedPath(event.target.value)}>
          {devices.map(device => <option key={device.path_hex} value={device.path_hex}>{lightingDeviceLabel(device)}</option>)}
        </SelectField>
      </div>}
      <fieldset className="lighting-controls" disabled={controlsLocked}>
        <div className="lighting-row">
          <label htmlFor="lighting-backlight">Backlight</label>
          <div className="lighting-backlight">
            <input id="lighting-backlight" type="range" min={0} max={100} step={1} value={backlight}
              onChange={event => setBacklightValue(Number(event.target.value))} />
            <span className="lighting-value" aria-hidden="true">{backlight}</span>
          </div>
        </div>
        <div className="lighting-row">
          <label htmlFor="lighting-colour">Colour</label>
          <div className="lighting-colour">
            <input id="lighting-colour" type="color" value={colour} onChange={event => setColourValue(event.target.value)} />
            <button type="button" className="button compact" onClick={saveFavourite}><Star size={15} />Save colour</button>
          </div>
        </div>
        {favourites.length > 0 && <div className="lighting-favourites" role="group" aria-label="Favourite colours">
          {favourites.map(hex => <span key={hex} className="lighting-favourite">
            <button type="button" className="swatch" style={{ background: hex }} title={hex}
              aria-label={`Use colour ${hex}`} onClick={() => setColourValue(hex)} />
            <button type="button" className="remove" aria-label={`Remove colour ${hex}`} onClick={() => dropFavourite(hex)}>
              <X size={11} />
            </button>
          </span>)}
        </div>}
      </fieldset>
      <div className="lighting-actions">
        <button type="button" className="button" disabled={controlsLocked || busy} onClick={() => { void reset(); }}>
          <RotateCcw size={16} />{busy ? 'Resetting…' : 'Reset'}
        </button>
      </div>
      {reason && <p className="fine-print lighting-reason" role="status">{reason}</p>}
      {error && <div className="service-failure" role="alert"><strong>Could not reach the dial.</strong> {error}</div>}
      {!error && status && <p className="fine-print" role="status">{status}</p>}
      <p className="fine-print">Changes apply the moment you adjust them. Reset hands the dial back to its built-in animation.</p>
      <details className="learn-more">
        <summary>Learn more</summary>
        <p>Applying lighting puts the dial in software mode — the mode profiles also run in. The dial’s built-in animation stops and its screen stays dark until a run draws on it; the backlight and colour take effect in this mode. Reset leaves software mode, and the built-in animation returns.</p>
        <p>Whole kit applies the same backlight and colour to every connected encoder dial. A dial that a run is driving is left out; stop the run to light it. Knob rings, base edge and fader lighting are not available yet.</p>
      </details>
    </div>
  </section>;
}
