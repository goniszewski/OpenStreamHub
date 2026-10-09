import { useSyncExternalStore } from 'react';
import type { CSSProperties } from 'react';
import { ChevronRight, RotateCw } from 'lucide-react';
import { EncoderDisplayContent, KeyDisplayContent } from './DisplayContent';
import { keypadDepth, keypadIsBack } from './keypad-model';
import { KeypadScreen } from './KeypadScreen';
import { knobOrientationLabels, knobOrientationStore, useKnobOrientation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import { knobControls, moduleArtwork, moduleImage } from './module-controls';
import { livePointer, livePressed, livePulse, RigPreview } from './RigPreview';
import { ServicePanel } from './ServicePanel';
import type { RunSession } from './ServicePanel';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import type { SessionState } from './profile-session';
import { controlCounts, controlLabel, moduleNote, modules } from './ui-model';
import type { Page } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

export function ModuleCatalog({ editor, feed }: { editor: DeviceEditor; feed?: DeviceFeed }) {
  const connected = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState);
  const connectedModules = new Set(connected.devices.map(device => device.module));
  return <aside className="catalog" aria-label="Module catalog"><h2>Modules</h2>
    {modules.map(module => <button key={module.id} className={`module-card ${editor.selectedModule === module.id ? 'selected' : ''}`} aria-pressed={editor.selectedModule === module.id} onClick={() => editor.chooseModule(module.id)}>
      <span className="module-photo"><img className={module.art === 'encoder' || module.art === 'knob' ? 'art-square' : 'art-wide'} {...moduleImage(module)} alt="" /></span>
      <span className="module-copy"><span className="module-name">{module.shortName}</span><small>{connectedModules.has(module.id) ? 'Connected' : 'Not connected'}</small></span>
    </button>)}
  </aside>;
}

const noopSubscribe = () => () => {};
const idleState = () => idleFeedState;

function HardwareStatus({ feed, desktop }: { feed?: DeviceFeed; desktop: boolean }) {
  const count = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState).devices.length;
  const connected = desktop && count > 0;
  return <p className="status"><span className={`status-dot${connected ? ' on' : ''}`} />{connected ? `${count} module${count === 1 ? '' : 's'} connected` : 'No hardware connected'}</p>;
}

// The middle column: the module image with its live input, the control pickers, then the run section.
// Testing a binding lives in the inspector (Test button and its result); starting and stopping a run
// lives in the header Run control.
export function DeviceWorkspace({ editor, state, desktop, disabled, setPage, feed, run }: { editor: DeviceEditor; state: SessionState; desktop: boolean; feed?: DeviceFeed; run?: RunSession; disabled: boolean; setPage(page: Page): void }) {
  const { selected, selection, isKeypad, keypadPath, currentKeypadPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  const knobOrientation = useKnobOrientation();
  const isKnob = selected.id === 'm003k';
  return <section className="workspace" aria-label="Device controls">
    <div className="page-heading"><h1>Your controls</h1><HardwareStatus feed={feed} desktop={desktop} /></div>
    <RigPreview editor={editor} state={state} feed={feed} />
    <HeroImage editor={editor} profile={profile} feed={feed} knobOrientation={knobOrientation} />
    {isKnob && <KnobOrientationControl orientation={knobOrientation} onRotate={knobOrientationStore.rotate} />}
    {selected.supported ? <>
      {isKeypad && <KeypadScreen pages={editor.keypadPages} allPages={profile.keypad!.pages} path={keypadPath} selected={selection.control} onSelect={control => editor.changeSelection({ ...selection, control })} onNavigate={editor.navigateKeypad} />}
      {!isKeypad && controlCounts[selection.module] > 1 && <div className="control-picker" role="group" aria-label="Control selection">
        {Array.from({ length: controlCounts[selection.module] }, (_, index) => index).map(control => <button key={control} className={selection.control === control ? 'selected' : ''} aria-pressed={selection.control === control} onClick={() => editor.changeSelection({ ...selection, control })}>{controlLabel(selection.module, control)}</button>)}
      </div>}
      <p className="hero-note">{moduleNote(selection.module)}</p>
    </> : <div className="planned-panel"><h2>Not available yet</h2><p>{selected.name} is in the module catalog, but this app cannot yet read its input or edit its bindings. See Compatibility for what it supports.</p><button className="button" onClick={() => setPage('compatibility')}>See compatibility<ChevronRight size={17} /></button></div>}
    <ServicePanel run={run} />
  </section>;
}

// The module image with its hit areas. It alone subscribes to the feed, so live input re-renders
// only this image, not the rest of the workspace. Live state is read-only and keyed by module type,
// as in the rig: same-type devices share one live state.
export function HeroImage({ editor, profile, feed, knobOrientation }: { editor: DeviceEditor; profile: SessionState['draft']['profile']; feed?: DeviceFeed; knobOrientation: KnobOrientation }) {
  const { selected, selection, isKeypad, keypadPath, currentKeypadPage, binding } = editor;
  const live = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleState, feed?.getState ?? idleState).live[selected.id];
  const isKnob = selected.id === 'm003k';
  const controls = isKnob ? knobControls(knobOrientation) : moduleArtwork[selected.id].controls;
  return <div className={`hero${isKeypad ? ' keypad-art' : ''} ${selected.art === 'knob' || selected.art === 'encoder' ? 'square' : 'wide'}${selected.id === 'm005s' || selected.id === 'm032le' ? ` art-${selected.art}` : ''}`}>
      <div className={`module-image${isKnob ? ' knob-image' : ''}`} style={{ '--art-aspect': moduleArtwork[selected.id].aspect } as CSSProperties} role="group" aria-label={`${selected.name} image controls`}>
        <img {...moduleImage(selected, knobOrientation)} alt="" draggable={false} />
        {selected.id === 'm131e' && <div className="single-display" style={{ left: '20%', top: '14%', width: '60%', height: '56%' }}><EncoderDisplayContent action={binding?.action} /></div>}
        {selected.id === 'm131e' && controls.map(area => <span key={area.control} className={`image-control round live-overlay${livePressed(live, area.control)}`} aria-hidden="true"
          style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}>{livePointer(live, area.control, true)}</span>)}
        {selected.supported && controlCounts[selected.id] > 1 && controls.map(area => <button key={area.control} type="button"
          className={`image-control${area.round ? ' round' : ''}${isKeypad ? ' key-display' : ''}${selection.control === area.control ? ' selected' : ''}${selected.id === 'm005s' ? (live?.faders[area.control] !== undefined ? ' live' : '') : livePressed(live, area.control)}`}
          style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}
          aria-label={`Select ${controlLabel(selected.id, area.control)} on module image`}
          aria-pressed={selection.control === area.control} title={controlLabel(selected.id, area.control)}
          onClick={() => editor.changeSelection({ ...selection, control: area.control })}>
          {isKeypad && <KeyDisplayContent key={currentKeypadPage.id} artwork={currentKeypadPage.artwork?.[String(area.control)]} slot={currentKeypadPage.slots[area.control]} pages={profile.keypad!.pages} depth={keypadDepth(keypadPath)} back={keypadIsBack(keypadPath, area.control)} />}
          {selected.id === 'm005s' && live?.faders[area.control] !== undefined && <span className="fader-thumb" style={{ bottom: `${live.faders[area.control]}%` }} />}
          {!isKeypad && selected.id !== 'm005s' && <>{livePointer(live, area.control)}{livePulse(live, area.control)}</>}
        </button>)}
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
