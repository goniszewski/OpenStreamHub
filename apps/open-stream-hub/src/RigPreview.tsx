import { useEffect, useState, useSyncExternalStore } from 'react';
import type { CSSProperties, PointerEvent } from 'react';
import { ChevronLeft, ChevronRight, ChevronUp, ChevronDown, X } from 'lucide-react';
import { idleFeedState, isRunning, writeAutoDetect } from './device-feed';
import type { DeviceFeed } from './device-feed';
import type { ModuleLive } from './live-state';
import { EncoderDisplayContent, KeyDisplayContent } from './DisplayContent';
import { keypadDepth, keypadIsBack } from './keypad-model';
import { useKnobOrientation } from './knob-orientation';
import { knobControls, moduleArtwork, moduleImage } from './module-controls';
import { addRigModule, boundControlCount, controlAction, defaultFaderPosition, faderFromKey, faderFromPointer, isDeviceItem, moveRigModule, readRig, removeRigModule, syncDeviceRig, writeRig } from './rig-model';
import type { RigItem } from './rig-model';
import type { SessionState } from './profile-session';
import { controlLabel, faderMaximum, gestures, modules, replayLabels } from './ui-model';
import type { LiveDevice, ReplayKind } from './contracts';
import type { ModuleId } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

const noopSubscribe = () => () => {};
const idleState = () => idleFeedState;
const demoKinds = Object.keys(replayLabels) as ReplayKind[];

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };

const noDevices: LiveDevice[] = [];

// With auto-detect on, the rig mirrors connected devices (ids `device:<path>`, session-only, never persisted).
export function useRig(devices: LiveDevice[] = noDevices, autoDetect = false) {
  const mirrored = autoDetect ? devices : noDevices;
  const [rig, setRig] = useState<RigItem[]>(() => syncDeviceRig(readRig(webStorage), mirrored));
  useEffect(() => { setRig(current => syncDeviceRig(current, mirrored)); }, [mirrored]);
  useEffect(() => { writeRig(webStorage, rig); }, [rig]);
  return {
    rig,
    add: (module: ModuleId) => setRig(current => addRigModule(current, module, `rig-${crypto.randomUUID()}`)),
    remove: (id: string) => setRig(current => removeRigModule(current, id)),
    move: (id: string, offset: -1 | 1) => setRig(current => moveRigModule(current, id, offset)),
  };
}

// Every module in the rig is drawn from the draft profile, so edits appear as they are made.
// Live input (when a feed is supplied) is read-only: it overrides what the artwork shows and runs no actions.
// Dragging a fader while a feed runs still only moves the simulated value; the live value wins on screen.
export function RigPreview({ editor, state, feed }: { editor: DeviceEditor; state: SessionState; feed?: DeviceFeed }) {
  const feedState = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState);
  const [source, setSource] = useState('off');
  const { rig, add, remove, move } = useRig(feedState.devices, feed !== undefined && feedState.desktop && feedState.autoDetect);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [faders, setFaders] = useState<Record<string, number>>({});
  const profile = state.draft.profile;
  const { selection } = editor;
  const knobOrientation = useKnobOrientation();
  const active = rig.find(item => item.id === activeId && item.module === selection.module)?.id ?? rig.find(item => item.module === selection.module)?.id;

  function select(item: RigItem, control: number) {
    setActiveId(item.id);
    editor.chooseModule(item.module);
    editor.changeSelection({ module: item.module, control, gesture: gestures[item.module][0], serial: '' });
  }

  function dragFader(item: RigItem, control: number, event: PointerEvent<HTMLElement>) {
    if (event.type === 'pointermove' && event.buttons !== 1) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const value = faderFromPointer(event.clientY - rect.top, rect.height, faderMaximum);
    setFaders(current => ({ ...current, [`${item.id}:${control}`]: value }));
    if (event.type === 'pointerdown') { event.currentTarget.setPointerCapture?.(event.pointerId); select(item, control); }
    if (active === item.id || event.type === 'pointerdown') editor.changeFaderPosition(value);
  }

  const faderValue = (item: RigItem, control: number) =>
    feedState.live[item.module]?.faders[control] ?? (item.id === active && selection.control === control ? editor.faderPosition : faders[`${item.id}:${control}`] ?? defaultFaderPosition);

  return <section className="rig" aria-label="Module rig">
    <div className="rig-heading"><h2>Rig</h2>
      <label className="sr-only" htmlFor="rig-add">Add module to rig</label>
      <select id="rig-add" className="rig-add" value="" onChange={event => { if (event.target.value) add(event.target.value as ModuleId); }}>
        <option value="">Add module…</option>{modules.filter(module => module.supported).map(module => <option key={module.id} value={module.id}>{module.shortName}</option>)}
      </select>
    </div>
    {feed && <LiveControls feed={feed} feedState={feedState} source={source} setSource={setSource} />}
    {rig.length === 0 ? <p className="fine-print">Add a module to arrange your setup.</p> : <ol className="rig-row" style={{ '--stage-h': `${rig.length <= 3 ? 104 : rig.length <= 5 ? 96 : 76}px` } as CSSProperties}>
      {rig.map((item, index) => {
        const module = modules.find(entry => entry.id === item.module)!;
        const art = moduleArtwork[item.module];
        const isKnob = item.module === 'm003k';
        const controls = isKnob ? knobControls(knobOrientation) : art.controls;
        const isActive = item.id === active;
        const bound = boundControlCount(profile, item.module);
        const live = feedState.live[item.module];
        return <li key={item.id} className={`rig-item${isActive ? ' active' : ''}${isDeviceItem(item) ? ' connected' : ''}`}>
          <div className="rig-stage" style={{ '--art-aspect': art.aspect } as CSSProperties} role="group" aria-label={`${module.name} in rig`}>
            <img {...moduleImage(module, knobOrientation)} alt="" draggable={false} />
            {item.module === 'm131e' && <div className="single-display" style={{ left: '20%', top: '14%', width: '60%', height: '56%' }}><EncoderDisplayContent action={controlAction(profile, 'm131e', 0)} /></div>}
            {controls.map(area => {
              const action = controlAction(profile, item.module, area.control);
              const style = { left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` };
              const picked = isActive && selection.control === area.control;
              const label = `Select ${controlLabel(item.module, area.control)} in rig`;
              if (item.module === 'm131e') return <button key={area.control} type="button" className={`image-control round${picked ? ' selected' : ''}${livePressed(live, area.control)}`} style={style} aria-label={label} aria-pressed={picked} onClick={() => select(item, area.control)}>{livePointer(live, area.control)}</button>;
              if (item.module === 'm4315') {
                const page = editor.currentKeypadPage;
                return <button key={area.control} type="button" className={`image-control key-display${picked ? ' selected' : ''}${livePressed(live, area.control)}`} style={style} aria-label={label} aria-pressed={picked} onClick={() => select(item, area.control)}>
                  <KeyDisplayContent slot={page.slots[area.control]} pages={profile.keypad?.pages ?? []} depth={keypadDepth(editor.keypadPath)} back={keypadIsBack(editor.keypadPath, area.control)} />
                </button>;
              }
              if (item.module === 'm005s') {
                const value = faderValue(item, area.control);
                return <div key={area.control} role="slider" tabIndex={0} className={`image-control fader${action ? ' bound' : ''}${picked ? ' selected' : ''}${live?.faders[area.control] !== undefined ? ' live' : ''}`} style={style}
                  aria-label={controlLabel(item.module, area.control)} aria-orientation="vertical" aria-valuemin={0} aria-valuemax={faderMaximum} aria-valuenow={value}
                  onPointerDown={event => dragFader(item, area.control, event)} onPointerMove={event => dragFader(item, area.control, event)}
                  onKeyDown={event => { const next = faderFromKey(event.key, value, faderMaximum); if (next !== null) { event.preventDefault(); select(item, area.control); editor.changeFaderPosition(next); } }}>
                  <span className="fader-thumb" style={{ bottom: `${value}%` }} />
                </div>;
              }
              return <button key={area.control} type="button" className={`image-control${area.round ? ' round' : ''}${action ? ' bound' : ''}${picked ? ' selected' : ''}${livePressed(live, area.control)}`} style={style} aria-label={label} aria-pressed={picked} onClick={() => select(item, area.control)}>
                {livePointer(live, area.control)}
                {livePulse(live, area.control)}
              </button>;
            })}
          </div>
          <div className="rig-caption"><span title={module.shortName}>{module.shortName}</span><small>{bound ? `${bound} bound` : 'No bindings'}{isDeviceItem(item) ? ' · connected' : ''}</small></div>
          <div className="rig-tools">
            <button type="button" className="icon-button" aria-label={`Move ${module.shortName} left`} disabled={index === 0} onClick={() => move(item.id, -1)}><ChevronLeft size={14} /></button>
            <button type="button" className="icon-button" aria-label={`Move ${module.shortName} right`} disabled={index === rig.length - 1} onClick={() => move(item.id, 1)}><ChevronRight size={14} /></button>
            <button type="button" className="icon-button" aria-label={`Remove ${module.shortName} from rig`} disabled={isDeviceItem(item)} title={isDeviceItem(item) ? 'Connected modules are removed by unplugging them' : undefined} onClick={() => remove(item.id)}><X size={14} /></button>
          </div>
        </li>;
      })}
    </ol>}
    <p className="fine-print">Arrangement is a preview only. Modules of the same type share their bindings and live input. Live input is read-only and no actions run.</p>
  </section>;
}

// Read-only live overlays, shared by the rig tiles and the Devices hero image.
export const livePressed = (live: ModuleLive | undefined, control: number) => live?.keysDown.includes(control) ? ' pressed' : '';
export const livePointer = (live: ModuleLive | undefined, control: number) => live?.angles[control] === undefined ? null :
  <span className="live-pointer" style={{ transform: `rotate(${live.angles[control]}deg)` }} aria-hidden="true" />;
export function livePulse(live: ModuleLive | undefined, control: number) {
  const pulse = live?.pulses[control];
  return pulse && <span key={pulse.tick} className={`live-pulse ${pulse.direction}`} aria-hidden="true">{pulse.direction === 'up' ? <ChevronUp size={16} /> : <ChevronDown size={16} />}</span>;
}

export function LiveControls({ feed, feedState, source, setSource }: { feed: DeviceFeed; feedState: ReturnType<DeviceFeed['getState']>; source: string; setSource(source: string): void }) {
  const running = isRunning(feedState);
  const failure = Object.values(feedState.feeds).find(entry => entry.state === 'failed');
  const enabled = feedState.desktop;
  const start = () => {
    if (source.startsWith('demo:')) void feed.startDemo(source.slice(5) as ReplayKind);
    else if (source.startsWith('live:')) void feed.startLive(source.slice(5));
  };
  const stop = () => Object.values(feedState.feeds).filter(entry => entry.state === 'running').forEach(entry => void feed.stop(entry.feedId));
  const status = running ? 'Receiving live input (read-only).' : feedState.error ?? (enabled ? 'Live input is off.' : 'Live input needs the desktop app. Launch it to read hardware.');
  return <div className="live-controls">
    <label className="sr-only" htmlFor="rig-live-source">Live input source</label>
    <select id="rig-live-source" value={source} disabled={!enabled || running} onChange={event => setSource(event.target.value)}>
      <option value="off">Live: Off</option>
      {demoKinds.map(kind => <option key={kind} value={`demo:${kind}`}>Demo: {replayLabels[kind]}</option>)}
      {feedState.devices.map(device => {
        const short = modules.find(entry => entry.id === device.module)?.shortName ?? device.module;
        return <option key={device.path_hex} value={`live:${device.path_hex}`}>{short} {device.serial_number ?? device.path_hex.slice(-6)}</option>;
      })}
    </select>
    {running ? <button type="button" className="button" onClick={stop}>Stop</button> : <button type="button" className="button" disabled={!enabled || source === 'off'} onClick={start}>Start</button>}
    {running && <span className="live-badge">Live</span>}
    {enabled && <label className="auto-detect"><input type="checkbox" checked={feedState.autoDetect} onChange={event => { writeAutoDetect(webStorage, event.target.checked); feed.setAutoDetect(event.target.checked); }} />Auto-detect modules</label>}
    {failure || (feedState.error && !running)
      ? <span className="live-status failed" role="alert">{failure?.error ?? feedState.error}</span>
      : <span className="live-status" role="status">{status}</span>}
  </div>;
}
