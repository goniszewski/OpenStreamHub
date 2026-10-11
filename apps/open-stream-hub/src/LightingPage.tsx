import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Grid2X2, Lightbulb, LightbulbOff, Palette, RotateCcw, Star, X } from 'lucide-react';
import { SelectField } from './controls';
import { idleFeedState } from './device-feed';
import { resolveSelected } from './module-selection';
import type { ModuleSelection } from './module-selection';
import type { DeviceFeed } from './device-feed';
import {
  addColourFavourite, appliedLedStatus, appliedStatus, applyDelayMs, applyLedLighting, applyLighting, clampBacklight, createApplyQueue,
  defaultBacklight, defaultColour, defaultEffect, ledCount, lightingBlockReason, lightingEffects, lightingKind,
  lightingScopeLabels, lightingTargets, readColourFavourites,
  removeColourFavourite, resetLighting, resetStatus, turnedOffLedStatus, writeColourFavourites,
} from './lighting-model';
import type { LightingDevice, LightingScope } from './lighting-model';
import { controlsFromRecord, emptyLightingState, matchedModuleColour, withDialLook, withLedLook } from './lighting-state';
import type { LightingStateStore } from './lighting-state';
import { errorMessage } from './ui-model';
import type { Invoke } from './profile-session';
import type { ServiceRunner } from './service-runner';

// The Lighting page: the encoder dial's backlight and one solid colour, and
// the knob and fader modules' colour, effect and brightness per LED, applied
// to the hardware as the controls move. Everything that
// decides — targets, gating, favourites, request shaping — lives in
// lighting-model; this component only holds the controls and debounces.
// What an apply left on each unit is recorded in the lighting state, which
// the rig lights its art with and a replug is answered from.
export function LightingPage({ desktop, invoke, devices, selection, runner, storage = null, lighting = emptyLightingState }: {
  desktop: boolean; invoke?: Invoke; devices: LightingDevice[]; selection: ModuleSelection; runner?: ServiceRunner;
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null; lighting?: LightingStateStore;
}) {
  const picked = useSyncExternalStore(selection.subscribe, selection.getState, selection.getState).path;
  const records = useSyncExternalStore(lighting.subscribe, lighting.getState, lighting.getState).records;
  const [scope, setScope] = useState<LightingScope>('module');
  // The picked module, falling back to the first dial and then the first
  // module, so a replug or a scope flip never leaves the picker empty-handed.
  const selected = resolveSelected(devices, picked);
  const kind = selected ? lightingKind(selected.module) : 'dial';
  const leds = selected ? ledCount(selected.module) : 0;
  const [ledIndex, setLedIndex] = useState<number | null>(null);
  // A LED pick only means something for the module it was made on, and only per module.
  const led = kind === 'led' && scope === 'module' && ledIndex !== null && ledIndex < leds ? ledIndex : null;
  // The controls start from what was last applied to the picked unit: that
  // LED's own look when one is picked, LED 1's for All LEDs, the dial's
  // backlight and colour. A unit never lit starts at the defaults. Read at
  // first render, and re-read when the pick or the record changes.
  const stored = controlsFromRecord(selected?.path_hex ? records[selected.path_hex] : undefined, kind ?? 'dial', led);
  const [backlight, setBacklight] = useState(stored?.backlight ?? defaultBacklight);
  const [colour, setColour] = useState(stored?.colour ?? defaultColour);
  const [effect, setEffect] = useState(stored?.effect ?? defaultEffect);
  const [favourites, setFavourites] = useState<string[]>(() => readColourFavourites(storage));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const targets = lightingTargets(devices, scope, selected?.path_hex ?? null);
  // Whole kit flags the modules it will reach on the shared strip.
  const markedKey = scope === 'kit' ? targets.map(device => device.path_hex).join(',') : '';
  useEffect(() => {
    selection.mark(markedKey ? markedKey.split(',') : []);
    return () => selection.mark([]);
  }, [selection, markedKey]);
  const selectedPath = selected?.path_hex ?? null;
  useEffect(() => {
    const look = controlsFromRecord(selectedPath ? records[selectedPath] : undefined, kind ?? 'dial', led);
    setEffect(look?.effect ?? defaultEffect);
    setColour(look?.colour ?? defaultColour);
    setBacklight(look?.backlight ?? defaultBacklight);
  }, [selectedPath, kind, led, records]);
  // A base takes no input, so no run can own it.
  const runOwned = targets.some(device => device.module !== 'm0064' && (runner?.isActivePath(device.path_hex, { ...device, module: device.module }) ?? false));
  const reason = lightingBlockReason({ desktop, runOwnedTargets: runOwned, scope, selected, devices });
  // The base's edge strip can take a module's colour: the first lit knob or
  // fader LED, or nothing to match when no module is lit.
  const matched = selected?.module === 'm0064' && scope === 'module' ? matchedModuleColour(records, devices) : null;
  // The apply queue reads the latest values and gate from this ref, not from
  // a render closure; a trailing apply runs after the state that scheduled it.
  const latest = useRef({ invoke, reason, targets, backlight, colour, kind, effect, led, lighting });
  latest.current = { invoke, reason, targets, backlight, colour, kind, effect, led, lighting };
  // One write at a time, and a change landing during a write re-applies once
  // with the latest values, so a dragged slider is never silently dropped —
  // even while a slow Whole kit write is running. Recreated per mount cycle:
  // StrictMode's effect/cleanup/effect must always leave a live queue.
  const applyRef = useRef<ReturnType<typeof createApplyQueue> | null>(null);
  useEffect(() => {
    const queue = createApplyQueue(async () => {
      const current = latest.current;
      if (current.reason !== null || !current.invoke || current.targets.length === 0) return;
      setError(null);
      try {
        const paths = current.targets.map(device => device.path_hex);
        if (current.kind === 'led') {
          const written = await applyLedLighting(current.invoke, {
            targets: paths, led: current.led, colour: current.colour, effect: current.effect, brightness: current.backlight,
          });
          current.lighting.record(paths, existing => withLedLook(existing, current.led, ledsFor(current.targets), {
            colour: current.colour, effect: current.effect, brightness: current.backlight,
          }));
          setStatus(appliedLedStatus(written));
        } else {
          const written = await applyLighting(current.invoke, { targets: paths, backlight: current.backlight, colour: current.colour });
          current.lighting.record(paths, existing => withDialLook(existing, current.backlight, current.colour));
          setStatus(appliedStatus(written));
        }
      } catch (reason) { setError(errorMessage(reason)); }
    }, applyDelayMs);
    applyRef.current = queue;
    return () => { applyRef.current = null; queue.dispose(); };
    // The queue reads everything through the latest ref; the store arrives with the app.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The LED count of the module kind the targets share (a Whole kit write
  // reaches one kind at a time).
  function ledsFor(targets: LightingDevice[]): number {
    return targets[0] ? ledCount(targets[0].module) : 0;
  }

  function scheduleApply() {
    applyRef.current?.schedule();
  }

  // The dial's reset leaves software mode, so nothing applied here is on it
  // any more: the record goes too. The LED modules have no built-in mode to
  // return to — turning them off is their reset, and the Off look is kept so
  // a replug restores it.
  async function reset() {
    const current = latest.current;
    if (current.reason !== null || !current.invoke || current.targets.length === 0 || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (current.kind === 'led') {
        const paths = current.targets.map(device => device.path_hex);
        const written = await applyLedLighting(current.invoke, {
          targets: paths, led: current.led, colour: current.colour, effect: 0, brightness: current.backlight,
        });
        current.lighting.record(paths, existing => withLedLook(existing, current.led, ledsFor(current.targets), {
          colour: current.colour, effect: 0, brightness: current.backlight,
        }));
        setEffect(0);
        setStatus(turnedOffLedStatus(written, current.led));
      } else {
        const written = await resetLighting(current.invoke, current.targets.map(device => device.path_hex));
        current.lighting.record(current.targets.map(device => device.path_hex), () => undefined);
        setStatus(resetStatus(written));
      }
    } catch (reason) { setError(errorMessage(reason)); }
    setBusy(false);
  }

  function setBacklightValue(value: number) {
    setBacklight(clampBacklight(value));
    scheduleApply();
  }

  function setEffectValue(value: number) {
    setEffect(value);
    scheduleApply();
  }

  function setLedValue(value: number | null) {
    setLedIndex(value);
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
    <div className="page-title"><div><h1>Lighting</h1><p>The encoder dial’s backlight and colour, and the knob and fader modules’ colour, effect and brightness, applied to the hardware as you adjust them and kept across a replug.</p></div></div>
    <div className="panel lighting-panel">
      <div className="theme-picker lighting-scope" role="group" aria-label="Lighting scope">
        {(Object.keys(lightingScopeLabels) as LightingScope[]).map(value => <button key={value} type="button"
          aria-pressed={scope === value} className={scope === value ? 'active' : ''} onClick={() => setScope(value)}>
          {value === 'module' ? <Lightbulb size={15} /> : <Grid2X2 size={15} />}{lightingScopeLabels[value]}
        </button>)}
      </div>
      <fieldset className="lighting-controls" disabled={controlsLocked}>
        {kind === 'led' && <div className="lighting-row">
          <label htmlFor="lighting-effect">Effect</label>
          <SelectField id="lighting-effect" value={String(effect)} onChange={event => setEffectValue(Number(event.target.value))}>
            {lightingEffects.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}
          </SelectField>
        </div>}
        {kind === 'led' && scope === 'module' && leds > 1 && <div className="lighting-row">
          <label htmlFor="lighting-led">LED</label>
          <SelectField id="lighting-led" value={led === null ? 'all' : String(led)}
            onChange={event => setLedValue(event.target.value === 'all' ? null : Number(event.target.value))}>
            <option value="all">All LEDs</option>
            {Array.from({ length: leds }, (_, index) => <option key={index} value={index}>LED {index + 1}</option>)}
          </SelectField>
        </div>}
        <div className="lighting-row">
          <label htmlFor="lighting-backlight">{kind === 'led' ? 'Brightness' : 'Backlight'}</label>
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
        {selected?.module === 'm0064' && <button type="button" className="button compact" disabled={controlsLocked || matched === null}
          title={matched === null ? 'No lit knob or fader LED to match yet' : `Copies ${matched} from the first lit knob or fader LED`}
          onClick={() => { if (matched !== null) setColourValue(matched); }}>
          <Palette size={16} />Match modules
        </button>}
        <button type="button" className="button" disabled={controlsLocked || busy} onClick={() => { void reset(); }}>
          {kind === 'led' ? <LightbulbOff size={16} /> : <RotateCcw size={16} />}{busy ? (kind === 'led' ? 'Turning off…' : 'Resetting…') : kind === 'led' ? 'Turn off' : 'Reset'}
        </button>
      </div>
      {reason && <p className="fine-print lighting-reason" role="status">{reason}</p>}
      {error && <div className="service-failure" role="alert"><strong>{kind === 'led' ? 'Could not reach the module.' : 'Could not reach the dial.'}</strong> {error}</div>}
      {!error && status && <p className="fine-print" role="status">{status}</p>}
      <p className="fine-print">Changes apply the moment you adjust them. {kind === 'led' ? 'Turn off switches the picked LED, or every LED, back to Off.' : 'Reset hands the dial back to its built-in animation.'}</p>
      <details className="learn-more">
        <summary>Learn more</summary>
        <p>Applying lighting puts the dial in software mode — the mode profiles also run in. The dial’s built-in animation stops and its screen stays dark until a run draws on it; the backlight and colour take effect in this mode. Reset leaves software mode, and the built-in animation returns.</p>
        <p>Whole kit applies the same backlight and colour to every connected encoder dial. While a run is driving any dial, Whole kit is unavailable. Stop the run to change lighting. Knob and fader modules take a colour, an effect and a brightness; pick one LED or all of them, and Whole kit reaches every knob and fader module and the base’s edge strip. Match modules copies the colour of the first lit knob or fader LED onto the base’s edge strip. Turn off switches the picked LED, or every LED of the module, to Off. What you applied is sent again when the module replugs, so its lighting survives a reconnect.</p>
      </details>
    </div>
  </section>;
}
