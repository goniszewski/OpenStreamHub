import { useSyncExternalStore } from 'react';
import type { CSSProperties, DragEvent } from 'react';
import { ArrowLeft, ChevronDown, ChevronRight, RotateCw, Trash2, TriangleAlert } from 'lucide-react';
import { ContextMenu } from './controls';
import { DialFace, KeyDisplayContent } from './DisplayContent';
import { isValidForControl, offersActionDrag, readActionDrag } from './action-picker-model';
import type { PickerActionType } from './action-picker-model';
import type { Profile, ServiceModule } from './contracts';
import { bindingWarnings } from './binding-model';
import { BindingsPanel } from './BindingsPanel';
import { profileBindingCount } from './keypad-model';
import { keypadDepth, keypadIsBack } from './keypad-model';
import { KeypadScreen } from './KeypadScreen';
import { knobOrientationLabels, knobOrientationStore, useKnobOrientation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import { ModuleArtImage, ModuleGlass } from './ModuleArt';
import { dialScreen, knobControls, moduleArtwork } from './module-controls';
import { livePointer, livePressed, livePulse } from './ModuleStage';
import { RigPreview } from './RigPreview';
import { ServicePanel } from './ServicePanel';
import type { RunSession } from './ServicePanel';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { emptyLightingState, kindLighting, litLedColours } from './lighting-state';
import type { LightingStateStore } from './lighting-state';
import type { ProfileSession, SessionState } from './profile-session';
import { controlCounts, controlLabel, moduleNote, modules } from './ui-model';
import type { Page } from './ui-model';
import { defaultFaderPosition } from './rig-model';
import { keyMenuItems } from './useDeviceEditor';
import type { DeviceEditor } from './useDeviceEditor';

const noopSubscribe = () => () => {};
const idleState = () => idleFeedState;

// The collapsible list of every binding in the profile, under the module canvas. Closed it shows
// only the counts (bindings and, in the accent pill, warnings); open it is the full searchable
// list. Controlled by the app so ⌘F can open it and focus the search field.
export function BindingsDrawer({ open, onToggle, profile, session, editor, disabled }: {
  open: boolean; onToggle(): void; profile: Profile; session: ProfileSession; editor: DeviceEditor; disabled: boolean;
}) {
  const warnings = bindingWarnings(profile).length;
  return <section className={`bindings-drawer panel${open ? ' open' : ''}`} aria-label="All bindings in this profile">
    <button type="button" className="drawer-toggle" aria-expanded={open} aria-controls="bindings-drawer-content" onClick={onToggle}>
      <ChevronDown size={17} aria-hidden="true" />Bindings <span className="small-label">{profileBindingCount(profile)} in this profile</span>
      {warnings > 0 && <span className="drawer-warnings"><TriangleAlert size={13} aria-hidden="true" />{warnings}</span>}
    </button>
    {open && <div id="bindings-drawer-content" className="drawer-content"><BindingsPanel profile={profile} session={session} editor={editor} disabled={disabled} /></div>}
  </section>;
}

function HardwareStatus({ feed, desktop }: { feed?: DeviceFeed; desktop: boolean }) {
  const count = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState).devices.length;
  const connected = desktop && count > 0;
  return <p className="status"><span className={`status-dot${connected ? ' on' : ''}`} />{connected ? `${count} module${count === 1 ? '' : 's'} connected` : 'No hardware connected'}</p>;
}

// The middle column: the module image with its live input, the control pickers, then the run section.
// Testing a binding lives in the inspector (Test button and its result); starting and stopping a run
// lives in the header Run control.
export function DeviceWorkspace({ editor, state, desktop, disabled, setPage, feed, run, mac = false, lighting = emptyLightingState, pickedPath = null, onPickUnit = () => {} }: { editor: DeviceEditor; state: SessionState; desktop: boolean; feed?: DeviceFeed; run?: RunSession; disabled: boolean; setPage(page: Page): void; mac?: boolean; lighting?: LightingStateStore;
  // The unit the shared pick names, and how a pick on the canvas reports its unit back.
  pickedPath?: string | null; onPickUnit?(path: string): void }) {
  const { selected, selection, isKeypad, keypadPath, currentKeypadPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  const knobOrientation = useKnobOrientation();
  const isKnob = selected.id === 'm003k';
  // Only the device list is read here (its identity changes only when a module joins or leaves), so live input
  // re-renders the canvas and not this workspace.
  const devices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, () => (feed?.getState() ?? idleFeedState).devices, () => idleFeedState.devices);
  // A connected module is edited where it sits on the base layout. Picking one that is not connected swaps the
  // layout for that module's standalone view, with a way back.
  const onBase = feed !== undefined && devices.some(device => device.module === selected.id);
  const goBack = () => {
    const target = devices.find(device => device.path_hex === pickedPath) ?? devices[0];
    if (!target) return;
    onPickUnit(target.path_hex);
    editor.chooseModule(target.module);
  };
  return <section className="workspace" aria-label="Device controls">
    <div className="page-heading"><h1>Your controls</h1><HardwareStatus feed={feed} desktop={desktop} /></div>
    <RigPreview editor={editor} state={state} feed={feed} lighting={lighting} layout={onBase} pickedPath={pickedPath} onPickUnit={onPickUnit}
      dropHandlers={(module, control) => controlDropHandlers(module, control, disabled ? undefined : type => editor.dropAction(module, control, type))} />
    {!onBase && devices.length > 0 && <div className="standalone-bar" role="status">
      <button type="button" className="button compact" onClick={goBack}><ArrowLeft size={15} aria-hidden="true" />Go back to the base layout</button>
      <span className="fine-print">{selected.shortName} is not connected, so it is shown on its own.</span>
    </div>}
    {!onBase && <HeroImage editor={editor} profile={profile} feed={feed} knobOrientation={knobOrientation} disabled={disabled} lighting={lighting} />}
    {isKnob && <KnobOrientationControl orientation={knobOrientation} onRotate={knobOrientationStore.rotate} />}
    {selected.supported ? <>
      {isKeypad && <KeypadScreen pages={editor.keypadPages} allPages={profile.keypad!.pages} path={keypadPath} selected={selection.control} onSelect={control => editor.changeSelection({ ...selection, control })} onNavigate={editor.navigateKeypad}
        onDropAction={disabled ? undefined : (control, type) => editor.dropAction('m4315', control, type)}
        onMoveKey={disabled ? undefined : editor.moveKey} onKeyContext={disabled ? undefined : editor.openKeyMenu} />}
      {!onBase && !isKeypad && controlCounts[selection.module] > 1 && <div className="control-picker" role="group" aria-label="Control selection">
        {Array.from({ length: controlCounts[selection.module] }, (_, index) => index).map(control => <button key={control} className={selection.control === control ? 'selected' : ''} aria-pressed={selection.control === control} onClick={() => editor.changeSelection({ ...selection, control })}>{controlLabel(selection.module, control)}</button>)}
      </div>}
      {!onBase && <p className="hero-note">{moduleNote(selection.module)}</p>}
    </> : <div className="planned-panel"><h2>Not available yet</h2><p>{selected.name} is in the module catalog, but this app cannot yet read its input or edit its bindings. See Compatibility for what it supports.</p><button className="button" onClick={() => setPage('compatibility')}>See compatibility<ChevronRight size={17} /></button></div>}
    <ServicePanel run={run} />
    {editor.keyMenu && <ContextMenu x={editor.keyMenu.x} y={editor.keyMenu.y} label={`Key ${editor.keyMenu.control + 1} menu`} items={keyMenuItems(editor, mac)} onClose={editor.closeKeyMenu} />}
  </section>;
}

// The drag handlers a control needs to accept one picker entry: dragover admits only action
// drags, drop assigns the action's default parameters. Disabled controls accept nothing.
export function controlDropHandlers(module: ServiceModule, control: number, drop: ((type: PickerActionType) => void) | undefined): {
  onDragOver?: (event: DragEvent<HTMLElement>) => void; onDrop?: (event: DragEvent<HTMLElement>) => void;
} {
  if (!drop) return {};
  return {
    onDragOver: event => { if (offersActionDrag(event.dataTransfer)) event.preventDefault(); },
    onDrop: event => {
      const type = readActionDrag(event.dataTransfer);
      if (!type || !isValidForControl(type, module)) return;
      event.preventDefault();
      drop(type);
    },
  };
}

// The module image with its hit areas. It alone subscribes to the feed, so live input re-renders
// only this image, not the rest of the workspace. Live state is read-only and keyed by module type,
// as in the rig: same-type devices share one live state. Controls accept a dragged picker entry.
// The lighting the Lighting page applied lights the knob rings and the fader glow strips.
export function HeroImage({ editor, profile, feed, knobOrientation, disabled = false, lighting = emptyLightingState }: { editor: DeviceEditor; profile: SessionState['draft']['profile']; feed?: DeviceFeed; knobOrientation: KnobOrientation; disabled?: boolean; lighting?: LightingStateStore }) {
  const { selected, selection, isKeypad, keypadPath, currentKeypadPage, binding } = editor;
  const feedState = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState);
  const live = feedState.live[selected.id];
  const lightingRecords = useSyncExternalStore(lighting.subscribe, lighting.getState, lighting.getState).records;
  const isKnob = selected.id === 'm003k';
  const controls = isKnob ? knobControls(knobOrientation) : moduleArtwork[selected.id].controls;
  // Each cap shows its live value, else the value being tried in the editor for the selected fader.
  const faderValues = selected.id === 'm005s' ? controls.map(area => live?.faders[area.control] ?? (selection.control === area.control ? editor.faderPosition : defaultFaderPosition)) : undefined;
  // One stored look per module kind lights the art: the first connected unit of the kind that has one.
  const stored = selected.id === 'm003k' || selected.id === 'm005s' ? kindLighting(lightingRecords, feedState.devices, selected.id) : undefined;
  return <div className={`hero${isKeypad ? ' keypad-art' : ''} ${selected.art === 'knob' || selected.art === 'encoder' ? 'square' : 'wide'}${selected.id === 'm005s' || selected.id === 'm032le' ? ` art-${selected.art}` : ''}`}>
      <div className={`module-image${isKnob ? ' knob-image' : ''}`} style={{ '--art-aspect': moduleArtwork[selected.id].aspect } as CSSProperties} role="group" aria-label={`${selected.name} image controls`}>
        <ModuleArtImage module={selected} orientation={knobOrientation} faders={faderValues}
          knobs={isKnob ? litLedColours(stored, 3) : undefined}
          glows={selected.id === 'm005s' ? litLedColours(stored, 5) : undefined} />
        {selected.id === 'm131e' && <div className="single-display" style={{ left: `${dialScreen.left}%`, top: `${dialScreen.top}%`, width: `${dialScreen.width}%`, height: `${dialScreen.height}%` }}><DialFace profile={profile} /></div>}
        {selected.id === 'm131e' && controls.map(area => <span key={area.control} className={`image-control round live-overlay${livePressed(live, area.control)}`} aria-hidden="true"
          style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}>{livePointer(live, area.control, true)}</span>)}
        {selected.supported && controlCounts[selected.id] > 1 && controls.map(area => <button key={area.control} type="button"
          className={`image-control${area.round ? ' round' : ''}${isKeypad ? ' key-display' : ''}${selection.control === area.control ? ' selected' : ''}${selected.id === 'm005s' ? (live?.faders[area.control] !== undefined ? ' live' : '') : livePressed(live, area.control)}`}
          style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}
          aria-label={`Select ${controlLabel(selected.id, area.control)} on module image`}
          aria-pressed={selection.control === area.control} title={controlLabel(selected.id, area.control)}
          onClick={() => editor.changeSelection({ ...selection, control: area.control })}
          {...controlDropHandlers(selected.id, area.control, disabled ? undefined : type => editor.dropAction(selected.id, area.control, type))}>
          {isKeypad && <KeyDisplayContent key={currentKeypadPage.id} artwork={currentKeypadPage.artwork?.[String(area.control)]} slot={currentKeypadPage.slots[area.control]} pages={profile.keypad!.pages} depth={keypadDepth(keypadPath)} back={keypadIsBack(keypadPath, area.control)} />}
          {!isKeypad && selected.id !== 'm005s' && <>{livePointer(live, area.control)}{livePulse(live, area.control)}</>}
        </button>)}
        <ModuleGlass module={selected.id} />
      </div>
    </div>;
}

// The image turns with the module; hit areas are recomputed so their contents stay upright.
export function KnobOrientationControl({ orientation, onRotate }: { orientation: KnobOrientation; onRotate(): void }) {
  return <div className="orientation-control" role="group" aria-label="3-knob module orientation">
    <button type="button" className="button compact" aria-label="Rotate module 90 degrees clockwise" onClick={onRotate}><RotateCw size={15} />Rotate module</button>
    <span className="orientation-label" aria-live="polite">{knobOrientationLabels[orientation]}</span>
  </div>;
}
