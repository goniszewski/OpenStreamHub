import { useSyncExternalStore } from 'react';
import type { CSSProperties } from 'react';
import { CheckCircle2, ChevronRight, CircleHelp, LoaderCircle, Play, RotateCcw, RotateCw } from 'lucide-react';
import { IntegerField } from './controls';
import { EncoderDisplayContent, KeyDisplayContent } from './DisplayContent';
import { keypadDepth, keypadIsBack } from './keypad-model';
import { KeypadScreen } from './KeypadScreen';
import { moduleArtwork } from './module-controls';
import { RigPreview } from './RigPreview';
import { ServicePanel } from './ServicePanel';
import type { ServiceRunner } from './service-runner';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { isUnsaved } from './profile-session';
import type { KeypadPreview, SessionState } from './profile-session';
import { actionLabel, controlCounts, controlLabel, faderMaximum, gestureLabels, gestures, moduleNote, modules } from './ui-model';
import type { Page } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

export function ModuleCatalog({ editor }: { editor: DeviceEditor }) {
  return <aside className="catalog" aria-label="Module catalog"><h2>Modules</h2>
    {modules.map(module => <button key={module.id} className={`module-card ${editor.selectedModule === module.id ? 'selected' : ''}`} aria-pressed={editor.selectedModule === module.id} onClick={() => editor.chooseModule(module.id)}>
      <span className="module-photo"><img className={module.art === 'encoder' || module.art === 'knob' ? 'art-square' : 'art-wide'} src={`/modules/${module.art}.png`} alt="" /></span>
      <span className="module-copy"><span className="module-name">{module.shortName}</span><small>{module.supported ? 'Device test pending' : 'Planned'}</small></span>
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

export function DeviceWorkspace({ editor, state, desktop, disabled, setPage, feed, runner }: { editor: DeviceEditor; state: SessionState; desktop: boolean; feed?: DeviceFeed; runner?: ServiceRunner; disabled: boolean; setPage(page: Page): void }) {
  const { selected, selection, isKeypad, keypadPath, currentKeypadPage, binding, serialScope, faderPosition } = editor;
  const { preview, keypadPreview } = state;
  const profile = state.draft.profile;
  const serialMissing = serialScope === 'exact' && !selection.serial;
  const previewRunning = preview.status === 'running' || keypadPreview.status === 'running';
  return <section className="workspace" aria-label="Device controls">
    <div className="page-heading"><h1>Your controls</h1><HardwareStatus feed={feed} desktop={desktop} /></div>
    <RigPreview editor={editor} state={state} feed={feed} />
    <div className={`hero${isKeypad ? ' keypad-art' : ''} ${selected.art === 'knob' || selected.art === 'encoder' ? 'square' : 'wide'}${selected.id === 'm005s' || selected.id === 'm032le' ? ` art-${selected.art}` : ''}`}>
      <div className={`module-image${selected.id === 'm003k' ? ' knob-image' : ''}`} style={{ '--art-aspect': moduleArtwork[selected.id].aspect } as CSSProperties} role="group" aria-label={`${selected.name} image controls`}>
        <img src={`/modules/${selected.art}.png`} alt="" draggable={false} />
        {selected.id === 'm131e' && <div className="single-display" style={{ left: '20%', top: '14%', width: '60%', height: '56%' }}><EncoderDisplayContent action={binding?.action} /></div>}
        {selected.supported && controlCounts[selected.id] > 1 && moduleArtwork[selected.id].controls.map(area => <button key={area.control} type="button"
          className={`image-control${area.round ? ' round' : ''}${isKeypad ? ' key-display' : ''}${selection.control === area.control ? ' selected' : ''}`}
          style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}
          aria-label={`Select ${controlLabel(selected.id, area.control)} on module image`}
          aria-pressed={selection.control === area.control} title={controlLabel(selected.id, area.control)}
          onClick={() => editor.changeSelection({ ...selection, control: area.control })}>
          {isKeypad && <KeyDisplayContent slot={currentKeypadPage.slots[area.control]} pages={profile.keypad!.pages} depth={keypadDepth(keypadPath)} back={keypadIsBack(keypadPath, area.control)} />}
        </button>)}
      </div>
    </div>
    {selected.supported ? <>
      {isKeypad && <KeypadScreen pages={editor.keypadPages} allPages={profile.keypad!.pages} path={keypadPath} selected={selection.control} onSelect={control => editor.changeSelection({ ...selection, control })} onNavigate={editor.navigateKeypad} />}
      {!isKeypad && controlCounts[selection.module] > 1 && <div className="control-picker" role="group" aria-label="Control selection">
        {Array.from({ length: controlCounts[selection.module] }, (_, index) => index).map(control => <button key={control} className={selection.control === control ? 'selected' : ''} aria-pressed={selection.control === control} onClick={() => editor.changeSelection({ ...selection, control })}>{controlLabel(selection.module, control)}</button>)}
      </div>}
      <p className="hero-note">{moduleNote(selection.module)}</p>
      <section className="simulation-panel"><div className="section-heading"><h2>Simulation</h2><span className="small-label">Planned actions only</span></div>
        {isKeypad ? <button className="button" disabled={disabled || keypadPreview.status === 'running' || serialMissing} onClick={() => { void editor.testKeypad(); }}><Play size={17} />Simulate key click</button> : selection.module === 'm005s' ? <div className="position-preview"><label htmlFor="fader-position">Simulated fader position</label><div className="position-inputs"><input id="fader-position" type="range" min={0} max={faderMaximum} step={1} value={faderPosition} disabled={disabled || preview.status === 'running'} onChange={event => editor.changeFaderPosition(Number(event.target.value))} /><label className="sr-only" htmlFor="fader-position-value">Fader position value</label><IntegerField id="fader-position-value" value={faderPosition} minimum={0} maximum={faderMaximum} disabled={disabled || preview.status === 'running'} onCommit={editor.changeFaderPosition} /></div><p className="fine-print">Position scale: 0–100. Preview resolves the volume through the service; physical endpoints still need checking.</p><button className="button" disabled={disabled || previewRunning || serialMissing} onClick={() => { void editor.testBinding(); }}><Play size={16} />Preview position</button></div> :
          <div className="event-buttons">{gestures[selection.module].map(gesture => <button className="button" key={gesture} disabled={disabled || previewRunning || serialMissing} onClick={() => { void editor.testBinding(gesture); }}>{gesture === 'clockwise' || gesture === 'right' ? <RotateCw size={17} /> : gesture === 'counter_clockwise' || gesture === 'left' ? <RotateCcw size={17} /> : <Play size={16} />}{gestureLabels[gesture]}</button>)}</div>}
        {isKeypad ? <div className="preview-result" role={keypadPreview.status === 'failed' ? 'alert' : 'status'}><KeypadResult preview={keypadPreview} /></div> : <div className="preview-result" role={preview.status === 'failed' ? 'alert' : 'status'}>
          {preview.status === 'idle' ? <><CircleHelp size={20} /><span>{desktop ? 'Choose a gesture to preview its binding through the service.' : 'Launch the desktop app to preview bindings.'}</span></> :
            preview.status === 'running' ? <><LoaderCircle size={20} className="spinning" /><span>Resolving binding…</span></> :
            preview.status === 'failed' ? <><CircleHelp size={20} /><span className="error-text">{preview.error}</span></> :
            <><CheckCircle2 size={22} /><span>{gestureLabels[preview.event.gesture]}{preview.event.value ? ` ${preview.event.value.raw}/${preview.event.value.maximum}` : ''}: {preview.event.action ? `Would ${actionLabel(preview.event.action).toLowerCase()}.` : 'No action is bound to this event.'}</span></>}
        </div>}
        <p className="fine-print">No volume changes, shortcuts or application launches are executed.</p>
      </section>
    </> : <div className="planned-panel"><h2>Input support is planned</h2><p>{selected.name} is included in the module catalog. Its input decoder and binding support are not implemented in this app.</p><p>Display uploads and physical device behavior still require validation.</p><button className="button" onClick={() => setPage('compatibility')}>See compatibility<ChevronRight size={17} /></button></div>}
    <ServicePanel runner={runner} feed={feed} profile={profile} profileId={isUnsaved(state.draft) ? null : state.draft.profileId} desktop={desktop} unsaved={isUnsaved(state.draft)} />
  </section>;
}

function KeypadResult({ preview }: { preview: KeypadPreview }) {
  if (preview.status === 'idle') return <><CircleHelp size={20} /><span>Simulate a complete press and release on the current page.</span></>;
  if (preview.status === 'running') return <><LoaderCircle size={20} className="spinning" /><span>Resolving key click…</span></>;
  if (preview.status === 'failed') return <><CircleHelp size={20} /><span className="error-text">{preview.error}</span></>;
  const navigation = preview.result.events.find(event => event.navigation)?.navigation;
  const actions = preview.result.events.filter(event => event.action);
  return <><CheckCircle2 size={22} /><span>{navigation ? `Navigated ${navigation.depth ? `to L${navigation.depth}` : 'to root'}.` : actions.length ? actions.map(event => `${gestureLabels[event.gesture]}: would ${actionLabel(event.action).toLowerCase()}`).join('; ') : 'No action is bound to this key click.'}</span></>;
}
