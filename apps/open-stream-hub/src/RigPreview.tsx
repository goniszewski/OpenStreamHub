import { useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { Info } from 'lucide-react';
import type { CSSProperties, FocusEvent, MouseEvent, PointerEvent } from 'react';
import { idleFeedState, isRunning, writeAutoDetect } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { baseGlow, emptyLightingState, kindLighting, lightMotion } from './lighting-state';
import type { LightingSnapshot, LightingStateStore } from './lighting-state';
import { baseColumns, baseRows, layoutExtent, overhangLabel, overhangOf, placementLabel, placementOf } from './base-layout';
import { useKnobOrientation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import { ModuleStage, livePointer, livePressed, livePulse } from './ModuleStage';
import type { DropHandlers } from './ModuleStage';
import { faderTrackInset } from './module-art';
import { defaultFaderPosition, faderFromPointer } from './rig-model';
import type { SessionState } from './profile-session';
import { faderMaximum, gestures, modules, replayLabels } from './ui-model';
import type { LiveDevice, Profile, ReplayKind } from './contracts';
import type { ModuleId } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

const noopSubscribe = () => () => {};
const idleState = () => idleFeedState;
const demoKinds = Object.keys(replayLabels) as ReplayKind[];

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };

const devicePlacement = (device: LiveDevice) => placementOf(device.module, device.base_cell!);
const deviceLocation = (device: LiveDevice) => {
  const placement = devicePlacement(device);
  const overhang = overhangLabel(overhangOf(placement));
  return `${placementLabel(placement)}${overhang ? ` (${overhang})` : ''}`;
};

// An info mark with an instant tooltip. The tooltip is drawn on the page body, so no tile's clipping can cut it off.
function InfoTip({ text }: { text: string }) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const show = (event: MouseEvent<HTMLElement> | FocusEvent<HTMLElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    setAt({ x: box.left + box.width / 2, y: box.top });
  };
  return <span className="base-layout-info" aria-label={text} role="img" onMouseEnter={show} onMouseLeave={() => setAt(null)}>
    <Info size={11} aria-hidden="true" />
    {at && typeof document !== 'undefined' && createPortal(<span className="info-tip" role="tooltip" style={{ left: at.x, top: at.y }}>{text}</span>, document.body)}
  </span>;
}

// The base layout is the editing canvas: every connected module sits where it is on the base and carries its own
// hit areas, so a control is picked where it is. `picked` is the one unit being edited (the editor holds a module
// kind, the shared pick names the unit). A module without a detected position stays reachable in a row beside the base.
function ConnectedBaseLayout({ devices, editor, profile, feedState, lightingRecords, knobOrientation, pickedPath, basePath, onPickUnit, dropHandlers }: {
  basePath: string | null;
  devices: LiveDevice[];
  editor: DeviceEditor;
  profile: Profile;
  feedState: ReturnType<DeviceFeed['getState']>;
  lightingRecords: LightingSnapshot['records'];
  knobOrientation: KnobOrientation;
  pickedPath: string | null;
  onPickUnit(path: string): void;
  dropHandlers?(module: ModuleId, control: number): DropHandlers;
}) {
  const [faders, setFaders] = useState<Record<string, number>>({});
  if (devices.length === 0) return null;
  const { selection } = editor;
  const positioned = devices.filter(device => device.base_cell);
  const unpositioned = devices.filter(device => !device.base_cell);
  const extent = layoutExtent(positioned.map(devicePlacement));
  const glow = baseGlow(basePath ? lightingRecords[basePath] : undefined);
  const glowStyle = glow ? { '--glow': glow.colour, '--glow-strength': glow.brightness / 100 } as CSSProperties : {};
  const gridStyle = { '--cols': extent.cols, '--rows': extent.rows } as CSSProperties;
  // The unit being edited: the shared pick when it is a unit of the editor's kind, else the first unit of that kind.
  const ofKind = devices.filter(device => device.module === selection.module);
  const activePath = (ofKind.find(device => device.path_hex === pickedPath) ?? ofKind[0])?.path_hex ?? null;

  const motionOf = (device: LiveDevice) => device.module === 'm003k' || device.module === 'm005s'
    ? lightMotion(kindLighting(lightingRecords, devices, device.module), device.module === 'm003k' ? 3 : 5) : null;
  const select = (device: LiveDevice, control: number) => {
    onPickUnit(device.path_hex);
    editor.chooseModule(device.module);
    editor.changeSelection({ module: device.module, control, gesture: gestures[device.module][0], serial: '' });
  };
  const faderValue = (device: LiveDevice, control: number) =>
    feedState.live[device.module]?.faders[control] ?? (device.path_hex === activePath && selection.control === control ? editor.faderPosition : faders[`${device.path_hex}:${control}`] ?? defaultFaderPosition);
  function dragFader(device: LiveDevice, control: number, event: PointerEvent<HTMLElement>) {
    if (event.type === 'pointermove' && event.buttons !== 1) return;
    const rect = event.currentTarget.getBoundingClientRect();
    // The hit area spans the whole slot; a cap's centre only travels the part between its stops.
    const inset = rect.height * faderTrackInset;
    const value = faderFromPointer(event.clientY - rect.top - inset, rect.height - 2 * inset, faderMaximum);
    setFaders(current => ({ ...current, [`${device.path_hex}:${control}`]: value }));
    if (event.type === 'pointerdown') { event.currentTarget.setPointerCapture?.(event.pointerId); select(device, control); }
    if (device.path_hex === activePath || event.type === 'pointerdown') editor.changeFaderPosition(value);
  }
  const stage = (device: LiveDevice) => {
    const stored = device.module === 'm003k' || device.module === 'm005s' ? kindLighting(lightingRecords, devices, device.module) : undefined;
    return <ModuleStage module={device.module} where="on the base" editor={editor} profile={profile} live={feedState.live[device.module]} stored={stored}
      knobOrientation={knobOrientation} picked={device.path_hex === activePath} faderValue={control => faderValue(device, control)}
      onSelect={control => select(device, control)} onFaderPointer={(control, event) => dragFader(device, control, event)}
      onFaderValue={(control, value) => editor.changeFaderPosition(value)} dropHandlers={dropHandlers && (control => dropHandlers(device.module, control))} />;
  };
  // Clicking a module's tile (its art, caption or empty space) picks the unit; its own controls pick themselves.
  const pickTile = (device: LiveDevice) => (event: MouseEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).closest('.image-control')) return;
    select(device, device.path_hex === activePath ? selection.control : 0);
  };
  const caption = (device: LiveDevice) => {
    const base = modules.find(module => module.id === device.module)?.shortName ?? device.module;
    // Identical modules are told apart by their order in the inventory: "15-key 1/2".
    const same = devices.filter(other => other.module === device.module);
    const name = same.length > 1 ? `${base} ${same.indexOf(device) + 1}/${same.length}` : base;
    // A plain label, not a control: the tile and its own controls pick the unit. The serial number sits behind an info mark.
    return <div className="base-layout-name" data-picked={device.path_hex === activePath}>
      <span>{name}</span>
      {device.serial_number && <InfoTip text={`Serial number ${device.serial_number}`} />}
      {device.module === 'm005s' && !feedState.live.m005s && <span className="base-layout-note"> · positions not read</span>}
    </div>;
  };

  return <section className="base-layout" aria-label="Connected modules on the MasterHUB base">
    <div className="base-layout-heading"><div><h3>Connected base layout</h3><p>USB-C ports at the top and left · click a module or one of its controls to edit it</p></div><span>{baseColumns} × {baseRows} cells</span></div>
    <div className="base-layout-frame">
      <div className="base-layout-grid" style={gridStyle} role="group" aria-label={`MasterHUB base layout: ${positioned.map(device => `${modules.find(module => module.id === device.module)?.shortName ?? device.module} at ${deviceLocation(device)}`).join(', ')}`}>
        <div className={`base-layout-base${glow?.motion ? ` fx-${glow.motion}` : ''}`} aria-hidden="true" style={{ ...glowStyle, gridColumn: `${1 - extent.col} / span ${baseColumns}`, gridRow: `${1 - extent.row} / span ${baseRows}` }} />
        {positioned.map(device => {
          const placement = devicePlacement(device);
          const overhangs = Object.values(overhangOf(placement)).some(count => count > 0);
          return <div key={device.path_hex} className={`base-layout-module${motionOf(device) ? ` fx-${motionOf(device)}` : ''}${overhangs ? ' overhangs' : ''}${device.path_hex === activePath ? ' selected' : ''}`} onClick={pickTile(device)}
            style={{ gridColumn: `${placement.col - extent.col + 1} / span ${placement.cols}`, gridRow: `${placement.row - extent.row + 1} / span ${placement.rows}` }}>
            {stage(device)}
            {caption(device)}
          </div>;
        })}
      </div>
    </div>
    {unpositioned.length > 0 && <div className="base-layout-unknown"><p>Position not detected</p>
      <div className="base-layout-extra">{unpositioned.map(device => <div key={device.path_hex} className={`base-layout-module loose${device.path_hex === activePath ? ' selected' : ''}`} onClick={pickTile(device)}>{stage(device)}{caption(device)}</div>)}</div>
    </div>}
  </section>;
}

// The live-input controls and the base layout. With `layout` off (the picked module is not connected, so the
// standalone module view is showing) only the live controls remain.
export function RigPreview({ editor, state, feed, lighting = emptyLightingState, layout = true, pickedPath = null, basePath = null, onPickUnit = () => {}, dropHandlers }: {
  editor: DeviceEditor; state: SessionState; feed?: DeviceFeed; lighting?: LightingStateStore; layout?: boolean;
  pickedPath?: string | null; basePath?: string | null; onPickUnit?(path: string): void; dropHandlers?(module: ModuleId, control: number): DropHandlers;
}) {
  const feedState = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState);
  const lightingRecords = useSyncExternalStore(lighting.subscribe, lighting.getState, lighting.getState).shown;
  const [source, setSource] = useState('off');
  const knobOrientation = useKnobOrientation();
  if (!feed) return null;
  return <section className="rig" aria-label="Module rig">
    <LiveControls feed={feed} feedState={feedState} source={source} setSource={setSource} />
    {layout && <ConnectedBaseLayout devices={feedState.devices} editor={editor} profile={state.draft.profile} feedState={feedState} lightingRecords={lightingRecords}
      knobOrientation={knobOrientation} pickedPath={pickedPath} basePath={basePath} onPickUnit={onPickUnit} dropHandlers={dropHandlers} />}
  </section>;
}

export { livePointer, livePressed, livePulse };

export function LiveControls({ feed, feedState, source, setSource }: { feed: DeviceFeed; feedState: ReturnType<DeviceFeed['getState']>; source: string; setSource(source: string): void }) {
  const running = isRunning(feedState);
  const failure = Object.values(feedState.feeds).find(entry => entry.state === 'failed');
  const enabled = feedState.desktop;
  const start = () => {
    if (source.startsWith('demo:')) void feed.startDemo(source.slice(5) as ReplayKind);
    else if (source.startsWith('live:')) void feed.startLive(source.slice(5));
  };
  const stop = () => Object.values(feedState.feeds).filter(entry => entry.state === 'running').forEach(entry => void feed.stop(entry.feedId));
  const status = running ? 'Receiving live input (read-only).' : feedState.error ?? (feedState.sessionModules.length ? 'Showing input from the running profile (read-only).' : enabled ? 'Live input is off.' : 'Live input needs the desktop app. Launch it to read hardware.');
  return <div className="live-controls">
    <label className="sr-only" htmlFor="rig-live-source">Live input source</label>
    <select id="rig-live-source" value={source} disabled={!enabled || running} onChange={event => setSource(event.target.value)}>
      <option value="off">Live: Off</option>
      {demoKinds.map(kind => <option key={kind} value={`demo:${kind}`}>Demo: {replayLabels[kind]}</option>)}
      {feedState.devices.map(device => {
        const short = modules.find(entry => entry.id === device.module)?.shortName ?? device.module;
        const position = device.base_cell ? ` · ${deviceLocation(device)} on base` : '';
        return <option key={device.path_hex} value={`live:${device.path_hex}`}>{short}{position} {device.serial_number ?? device.path_hex.slice(-6)}</option>;
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
