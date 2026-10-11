import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Grid2X2, Lightbulb, RotateCcw, Star, X } from 'lucide-react';
import { SelectField } from './controls';
import { idleFeedState } from './device-feed';
import { useBaseModules } from './base-modules';
import type { DeviceFeed } from './device-feed';
import {
  addColourFavourite, appliedLedStatus, appliedStatus, applyDelayMs, applyLedLighting, applyLighting, clampBacklight, createApplyQueue,
  defaultBacklight, defaultColour, defaultEffect, ledCount, lightingBlockReason, lightingCapability, lightingDeviceLabel, lightingModuleInfo, lightingEffects, lightingKind,
  lightingScopeLabels, lightingTargets, readColourFavourites,
  removeColourFavourite, resetLighting, resetStatus, writeColourFavourites,
} from './lighting-model';
import type { LightingDevice, LightingScope } from './lighting-model';
import { errorMessage } from './ui-model';
import type { Invoke } from './profile-session';
import type { ServiceRunner } from './service-runner';

const noopSubscribe = () => () => {};
const idleState = () => idleFeedState;

// The Lighting page: the encoder dial's backlight and one solid colour, and
// the knob and fader modules' colour, effect and brightness per LED, applied
// to the hardware as the controls move. Everything that
// decides — targets, gating, favourites, request shaping — lives in
// lighting-model; this component only holds the controls and debounces.
export function LightingPage({ desktop, invoke, feed, runner, storage = null }: {
  desktop: boolean; invoke?: Invoke; feed?: DeviceFeed; runner?: ServiceRunner;
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null;
}) {
  const feedDevices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState).devices;
  const bases = useBaseModules(invoke, feedDevices.map(device => device.path_hex).join(','));
  // The base first: it is the foundation the modules sit on.
  const devices: LightingDevice[] = [...bases, ...feedDevices];
  const [scope, setScope] = useState<LightingScope>('module');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [backlight, setBacklight] = useState(defaultBacklight);
  const [colour, setColour] = useState(defaultColour);
  const [effect, setEffect] = useState(defaultEffect);
  const [ledIndex, setLedIndex] = useState<number | null>(null);
  const [favourites, setFavourites] = useState<string[]>(() => readColourFavourites(storage));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  // The picked module, falling back to the first dial and then the first
  // module, so a replug or a scope flip never leaves the picker empty-handed.
  const selected = devices.find(device => device.path_hex === selectedPath)
    ?? devices.find(device => device.module === 'm131e') ?? devices[0];
  const targets = lightingTargets(devices, scope, selected?.path_hex ?? null);
  const kind = selected ? lightingKind(selected.module) : 'dial';
  const leds = selected ? ledCount(selected.module) : 0;
  // A LED pick only means something for the module it was made on, and only per module.
  const led = kind === 'led' && scope === 'module' && ledIndex !== null && ledIndex < leds ? ledIndex : null;
  // A base takes no input, so no run can own it.
  const runOwned = targets.some(device => device.module !== 'm0064' && (runner?.isActivePath(device.path_hex, { ...device, module: device.module }) ?? false));
  const reason = lightingBlockReason({ desktop, runOwnedTargets: runOwned, scope, selected, devices });
  // The apply queue reads the latest values and gate from this ref, not from
  // a render closure; a trailing apply runs after the state that scheduled it.
  const latest = useRef({ invoke, reason, targets, backlight, colour, kind, effect, led });
  latest.current = { invoke, reason, targets, backlight, colour, kind, effect, led };
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
          setStatus(appliedLedStatus(written));
        } else {
          const written = await applyLighting(current.invoke, { targets: paths, backlight: current.backlight, colour: current.colour });
          setStatus(appliedStatus(written));
        }
      } catch (reason) { setError(errorMessage(reason)); }
    }, applyDelayMs);
    applyRef.current = queue;
    return () => { applyRef.current = null; queue.dispose(); };
  }, []);

  function scheduleApply() {
    applyRef.current?.schedule();
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
    <div className="page-title"><div><h1>Lighting</h1><p>The encoder dial’s backlight and colour, and the knob and fader modules’ colour, effect and brightness, applied to the hardware as you adjust them.</p></div></div>
    <div className="panel lighting-panel">
      <div className="theme-picker lighting-scope" role="group" aria-label="Lighting scope">
        {(Object.keys(lightingScopeLabels) as LightingScope[]).map(value => <button key={value} type="button"
          aria-pressed={scope === value} className={scope === value ? 'active' : ''} onClick={() => setScope(value)}>
          {value === 'module' ? <Lightbulb size={15} /> : <Grid2X2 size={15} />}{lightingScopeLabels[value]}
        </button>)}
      </div>
      <div className="lighting-modules" role="group" aria-label="Module to light">
        {devices.length === 0 && <p className="fine-print">No modules are connected.</p>}
        {devices.map(device => {
          const info = lightingModuleInfo(device.module);
          const picked = device.path_hex === selected?.path_hex;
          const included = scope === 'kit' && targets.some(target => target.path_hex === device.path_hex);
          const lit = lightingKind(device.module) !== null;
          return <button key={device.path_hex} type="button" aria-pressed={picked} aria-label={lightingDeviceLabel(device)}
            className={`lighting-tile${picked ? ' picked' : ''}${included ? ' included' : ''}${lit ? '' : ' unlit'}`}
            onClick={() => setSelectedPath(device.path_hex)}>
            <span className="lighting-tile-photo">{info.image && <img {...info.image} alt="" draggable={false} />}</span>
            <span className="lighting-tile-name">{info.name}</span>
            {device.serial_number && <span className="lighting-tile-serial">…{device.serial_number.slice(-4)}</span>}
            <span className="lighting-tile-capability">{included ? 'Included in Whole kit' : lightingCapability(device.module)}</span>
          </button>;
        })}
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
        {kind !== 'led' && <button type="button" className="button" disabled={controlsLocked || busy} onClick={() => { void reset(); }}>
          <RotateCcw size={16} />{busy ? 'Resetting…' : 'Reset'}
        </button>}
      </div>
      {reason && <p className="fine-print lighting-reason" role="status">{reason}</p>}
      {error && <div className="service-failure" role="alert"><strong>{kind === 'led' ? 'Could not reach the module.' : 'Could not reach the dial.'}</strong> {error}</div>}
      {!error && status && <p className="fine-print" role="status">{status}</p>}
      <p className="fine-print">Changes apply the moment you adjust them. {kind === 'led' ? 'Pick Off or another effect to change a module’s LEDs; there is no reset for them yet.' : 'Reset hands the dial back to its built-in animation.'}</p>
      <details className="learn-more">
        <summary>Learn more</summary>
        <p>Applying lighting puts the dial in software mode — the mode profiles also run in. The dial’s built-in animation stops and its screen stays dark until a run draws on it; the backlight and colour take effect in this mode. Reset leaves software mode, and the built-in animation returns.</p>
        <p>Whole kit applies the same backlight and colour to every connected encoder dial. While a run is driving any dial, Whole kit is unavailable. Stop the run to change lighting. Knob and fader modules take a colour, an effect and a brightness; pick one LED or all of them, and Whole kit reaches every knob and fader module. The base edge strip is not available here yet.</p>
      </details>
    </div>
  </section>;
}
