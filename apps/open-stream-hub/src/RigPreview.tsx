import { useEffect, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { Info } from 'lucide-react';
import type { CSSProperties, FocusEvent, MouseEvent, PointerEvent } from 'react';
import { idleFeedState, isRunning, writeAutoDetect } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { baseGlow, emptyLightingState, kindLighting, lightMotion } from './lighting-state';
import type { LightingSnapshot, LightingStateStore } from './lighting-state';
import { baseColumns, baseRows, layoutExtent, overhangLabel, overhangOf, placementLabel, placementOf } from './base-layout';
import { readFaderMemory, writeFaderMemory } from './fader-memory';
import { baseOrientationLabels, baseOrientationStore, baseOrientations, turnPlacement, usbEdges, turnedBase, turnedKnobOrientation, useBaseOrientation } from './base-orientation';
import type { BaseOrientation } from './base-orientation';
import type { BasePins } from './base-pins';
import { basePinsStore, pinLayout, useBasePins } from './base-pins';
import { footprintSwaps, moduleRotation, moduleRotationId, moduleRotationStore, useModuleRotations } from './module-rotation';
import { useKnobOrientation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import { SelectField } from './controls';
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

// The base's pogo-pin pads drawn on the bare base, in cell units so they scale with the layout.
function PogoPins({ cols, rows, baseOrientation, pins }: { cols: number; rows: number; baseOrientation: BaseOrientation; pins: BasePins }) {
  const layout = pinLayout(baseOrientation, pins);
  const pad = 0.2;
  const dot = 0.028;
  return <svg className="base-pogo-pins" viewBox={`0 0 ${cols} ${rows}`} preserveAspectRatio="none" data-testid="pogo-pins">
    {layout.pads.map(([u, v], index) => <g key={index} className="pogo-pad" transform={`translate(${u * cols} ${v * rows})`}>
      <rect x={-pad / 2} y={-pad / 2} width={pad} height={pad} rx={0.02} />
      {[-1, 1].flatMap(x => [-1, 1].map(y => <circle key={`${x}${y}`} cx={x * pad * 0.22} cy={y * pad * 0.22} r={dot} />))}
    </g>)}
    {layout.studs.map(([u, v], index) => <circle key={index} className="pogo-stud" cx={u * cols} cy={v * rows} r={0.06} />)}
    <rect className="pogo-bus" x={layout.bus.x0 * cols} y={layout.bus.y0 * rows} width={(layout.bus.x1 - layout.bus.x0) * cols} height={(layout.bus.y1 - layout.bus.y0) * rows} rx={0.02} />
  </svg>;
}

// The base layout is the editing canvas: every connected module sits where it is on the base and carries its own
// hit areas, so a control is picked where it is. `picked` is the one unit being edited (the editor holds a module
// kind, the shared pick names the unit). A module without a detected position stays reachable in a row beside the base.
function ConnectedBaseLayout({ devices, editor, profile, feedState, lightingRecords, knobOrientation, baseOrientation, pickedPath, basePath, onPickUnit, dropHandlers }: {
  basePath: string | null;
  devices: LiveDevice[];
  editor: DeviceEditor;
  profile: Profile;
  feedState: ReturnType<DeviceFeed['getState']>;
  lightingRecords: LightingSnapshot['records'];
  knobOrientation: KnobOrientation;
  baseOrientation: BaseOrientation;
  pickedPath: string | null;
  onPickUnit(path: string): void;
  dropHandlers?(module: ModuleId, control: number): DropHandlers;
}) {
  const [faders, setFaders] = useState<Record<string, number>>({});
  // The last positions the faders reported, remembered across launches; shown until a fader moves again.
  const storage = typeof localStorage === 'undefined' ? null : localStorage;
  const [lastSeen, setLastSeen] = useState<number[] | null>(() => readFaderMemory(storage));
  const liveFaders = feedState.live.m005s?.faders;
  const liveKey = liveFaders && [0, 1, 2, 3, 4].every(at => typeof liveFaders[at] === 'number') ? [0, 1, 2, 3, 4].map(at => liveFaders[at]).join(',') : '';
  useEffect(() => {
    if (!liveKey) return;
    const positions = liveKey.split(',').map(Number);
    if (positions.length !== 5 || positions.some(value => !Number.isFinite(value))) return;
    setLastSeen(positions);
    writeFaderMemory(storage, positions);
  }, [liveKey, storage]);
  if (devices.length === 0) return null;
  const { selection } = editor;
  const positioned = devices.filter(device => device.base_cell);
  const unpositioned = devices.filter(device => !device.base_cell);
  const pins = useBasePins();
  const rotations = useModuleRotations();
  const ownRotation = (device: LiveDevice) => rotations[moduleRotationId(device)] ?? null;
  const rotationOf = (device: LiveDevice) => moduleRotation(baseOrientation, ownRotation(device));
  // A module's cell follows the base; one turned a quarter against the base keeps its corner and swaps its width and height.
  const turnedPlacement = (device: LiveDevice) => {
    const placed = turnPlacement(devicePlacement(device), baseOrientation);
    return footprintSwaps(baseOrientation, ownRotation(device)) ? { ...placed, cols: placed.rows, rows: placed.cols } : placed;
  };
  const size = turnedBase(baseOrientation);
  const extent = layoutExtent(positioned.map(turnedPlacement), size);
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
    feedState.live[device.module]?.faders[control] ?? (device.path_hex === activePath && selection.control === control ? editor.faderPosition : faders[`${device.path_hex}:${control}`] ?? (device.module === 'm005s' ? lastSeen?.[control] : undefined) ?? defaultFaderPosition);
  function dragFader(device: LiveDevice, control: number, event: PointerEvent<HTMLElement>) {
    if (event.type === 'pointermove' && event.buttons !== 1) return;
    const rect = event.currentTarget.getBoundingClientRect();
    // The hit area spans the whole slot; a cap's centre only travels the part between its stops. The track's top end is
    // where the base's turn puts it: the top, the right, the bottom or the left.
    const turned = rotationOf(device);
    const length = turned === 90 || turned === 270 ? rect.width : rect.height;
    const inset = length * faderTrackInset;
    const along = { 0: event.clientY - rect.top, 90: rect.right - event.clientX, 180: rect.bottom - event.clientY, 270: event.clientX - rect.left }[turned];
    const value = faderFromPointer(along - inset, length - 2 * inset, faderMaximum);
    setFaders(current => ({ ...current, [`${device.path_hex}:${control}`]: value }));
    if (event.type === 'pointerdown') { event.currentTarget.setPointerCapture?.(event.pointerId); select(device, control); }
    if (device.path_hex === activePath || event.type === 'pointerdown') editor.changeFaderPosition(value);
  }
  const stage = (device: LiveDevice) => {
    const stored = device.module === 'm003k' || device.module === 'm005s' ? kindLighting(lightingRecords, devices, device.module) : undefined;
    return <ModuleStage module={device.module} where="on the base" editor={editor} profile={profile} live={feedState.live[device.module]} stored={stored}
      knobOrientation={turnedKnobOrientation(knobOrientation, rotationOf(device))} rotation={rotationOf(device)} picked={device.path_hex === activePath} faderValue={control => faderValue(device, control)}
      onSelect={control => select(device, control)} onFaderPointer={(control, event) => dragFader(device, control, event)}
      onFaderValue={(control, value) => editor.changeFaderPosition(value)} dropHandlers={dropHandlers && (control => dropHandlers(device.module, control))} />;
  };
  // Clicking a module's tile (its art, caption or empty space) picks the unit; its own controls pick themselves.
  const pickTile = (device: LiveDevice) => (event: MouseEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).closest('.image-control')) return;
    select(device, device.path_hex === activePath ? selection.control : 0);
  };
  const nameOf = (device: LiveDevice) => {
    const base = modules.find(module => module.id === device.module)?.shortName ?? device.module;
    // Identical modules are told apart by their order in the inventory: "15-key 1/2".
    const same = devices.filter(other => other.module === device.module);
    return same.length > 1 ? `${base} ${same.indexOf(device) + 1}/${same.length}` : base;
  };
  const pickedDevice = devices.find(device => device.path_hex === activePath) ?? null;
  const caption = (device: LiveDevice) => {
    const name = nameOf(device);
    // A plain label, not a control: the tile and its own controls pick the unit. The serial number sits behind an info mark.
    return <div className="base-layout-name" data-picked={device.path_hex === activePath}>
      <span>{name}</span>
      {device.serial_number && <InfoTip text={`Serial number ${device.serial_number}`} />}
      {device.module === 'm005s' && !feedState.live.m005s && <span className="base-layout-note">{lastSeen ? ' · last known positions' : ' · positions not read'}</span>}
    </div>;
  };

  return <section className="base-layout" aria-label="Connected modules on the MasterHUB base">
    <div className="base-layout-heading"><div><h3>Connected base layout</h3><p>USB-C ports at the {usbEdges[baseOrientation]} · click a module or one of its controls to edit it</p></div><span>{baseColumns} × {baseRows} cells</span>
      <div className="base-pin-controls" role="group" aria-label="Pogo pins">
        <button type="button" className="mode-choice" aria-pressed={pins.show} onClick={() => basePinsStore.set({ show: !pins.show })}>Pogo pins</button>
        {pins.show && <><button type="button" className="mode-choice" aria-pressed={pins.turned} title="Turn the pins half way round if they do not match how your base is seated" onClick={() => basePinsStore.set({ turned: !pins.turned })}>Turn 180°</button>
        <button type="button" className="mode-choice" aria-pressed={pins.mirrored} title="Mirror the pins if they do not match how your base is seated" onClick={() => basePinsStore.set({ mirrored: !pins.mirrored })}>Mirror</button></>}
      </div>
      <div className="mode-control base-orientation" role="group" aria-label="Base rotation">{baseOrientations.map(value => <button key={value} type="button" className="mode-choice" aria-pressed={value === baseOrientation} onClick={() => baseOrientationStore.set(value)}>{baseOrientationLabels[value]}</button>)}</div></div>
    {pickedDevice && <div className="base-layout-rotation" role="group" aria-label="Rotation of the selected module">
      <label>Rotation of {nameOf(pickedDevice)}
        <SelectField value={ownRotation(pickedDevice) === null ? 'follow' : String(ownRotation(pickedDevice))} onChange={event => moduleRotationStore.set(moduleRotationId(pickedDevice), event.target.value === 'follow' ? null : Number(event.target.value) as BaseOrientation)}>
          <option value="follow">Follow base ({baseOrientationLabels[baseOrientation]})</option>
          {baseOrientations.map(value => <option key={value} value={value}>{baseOrientationLabels[value]}</option>)}
        </SelectField>
      </label>
      {ownRotation(pickedDevice) !== null && <><span className="base-layout-overridden">Overridden</span><button type="button" className="button" onClick={() => moduleRotationStore.set(moduleRotationId(pickedDevice), null)}>Follow base</button></>}
    </div>}
    <div className="base-layout-frame">
      <div className="base-layout-grid" style={gridStyle} role="group" aria-label={`MasterHUB base layout: ${positioned.map(device => `${modules.find(module => module.id === device.module)?.shortName ?? device.module} at ${deviceLocation(device)}`).join(', ')}`}>
        <div className={`base-layout-base${glow?.motion ? ` fx-${glow.motion}` : ''}`} aria-hidden="true" style={{ ...glowStyle, gridColumn: `${1 - extent.col} / span ${size.cols}`, gridRow: `${1 - extent.row} / span ${size.rows}` }}>
          {pins.show && <PogoPins cols={size.cols} rows={size.rows} baseOrientation={baseOrientation} pins={pins} />}
        </div>
        {positioned.map(device => {
          const placement = turnedPlacement(device);
          const overhangs = Object.values(overhangOf(devicePlacement(device))).some(count => count > 0);
          return <div key={device.path_hex} className={`base-layout-module${motionOf(device) ? ` fx-${motionOf(device)}` : ''}${overhangs ? ' overhangs' : ''}${device.path_hex === activePath ? ' selected' : ''}`} onClick={pickTile(device)}
            style={{ gridColumn: `${placement.col - extent.col + 1} / span ${placement.cols}`, gridRow: `${placement.row - extent.row + 1} / span ${placement.rows}` }}>
            {stage(device)}
            {caption(device)}
          </div>;
        })}
      </div>
    </div>
    {unpositioned.length > 0 && <div className="base-layout-unknown"><p>Position not detected{unpositioned.some(device => device.port_chain) && <span className="base-layout-note"> · USB port chain {unpositioned.map(device => device.port_chain && `${device.port_chain}`).filter(Boolean).join(', ')}</span>}</p>
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
  const baseOrientation = useBaseOrientation();
  if (!feed) return null;
  return <section className="rig" aria-label="Module rig">
    <LiveControls feed={feed} feedState={feedState} source={source} setSource={setSource} />
    {layout && <ConnectedBaseLayout devices={feedState.devices} editor={editor} profile={state.draft.profile} feedState={feedState} lightingRecords={lightingRecords}
      knobOrientation={knobOrientation} baseOrientation={baseOrientation} pickedPath={pickedPath} basePath={basePath} onPickUnit={onPickUnit} dropHandlers={dropHandlers} />}
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
    <select id="rig-live-source" value={running && source === 'off' ? 'running' : source} disabled={!enabled || running} onChange={event => setSource(event.target.value)}>
      <option value="off">Live: Off</option>
      {running && source === 'off' && <option value="running">Live: Auto-detected</option>}
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
