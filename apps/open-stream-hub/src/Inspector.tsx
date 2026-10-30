import { useEffect, useState } from 'react';
import { english, useT } from './i18n-react';
import type { Translate } from './i18n-react';
import { CheckCircle2, CircleHelp, Folder, LoaderCircle, Copy, ClipboardPaste, Play, Trash2 } from 'lucide-react';
import type { Gesture, Profile } from './contracts';
import { DialDisplayPanel } from './DialDisplayPanel';
import { ScreenBrightnessControl } from './ScreenBrightnessControl';
import { ActionFields } from './ActionFields';
import { VariablesPanel } from './VariableFields';
import { VariablesContext } from './variables-context';
import { renameVariable, variableUseCounts } from './variable-model';
import { IntegerField, LabelError, SelectField } from './controls';
import { keypadDepth, keypadMaximumDepth, keypadMaximumPages, renameKeypadAction, renameKeypadPage } from './keypad-model';
import { useObsCatalog } from './obs-catalog';
import type { ObsCatalogStore } from './obs-catalog';
import { useStreamlabsCatalog } from './streamlabs-catalog';
import type { StreamlabsCatalogStore } from './streamlabs-catalog';
import type { KeypadPreview, Invoke, ProfileSession, SessionState } from './profile-session';
import { KeyArtworkEditor } from './KeyArtworkEditor';
import { actionLabel, controlLabel, faderMaximum, gestureLabels, gestures } from './ui-model';
import type { Page } from './ui-model';
import type { DeviceEditor, SerialScope } from './useDeviceEditor';
import { ControlWarnings } from './BindingsPanel';
import { hint, isMacPlatform, withHint } from './shortcuts';
import { editorGestures } from './binding-model';
import { DialFunctionsPanel, bindingPreviewText } from './DialFunctionsPanel';
import { emptyFaderStats, faderStatsText, recordFader } from './fader-readout';
import type { FaderStats } from './fader-readout';


export function Inspector({ editor, session, state, disabled, desktop = false, invoke, setPage, catalog: catalogStore, streamlabs: streamlabsStore, liveFader }: { editor: DeviceEditor; session: ProfileSession; state: SessionState; disabled: boolean; desktop?: boolean; invoke?: Invoke; setPage?: (page: Page) => void; catalog?: ObsCatalogStore; streamlabs?: StreamlabsCatalogStore; liveFader?: number }) {
  const t = useT();
  const mac = isMacPlatform(typeof navigator === 'undefined' ? undefined : navigator);
  const { selected, selection, isKeypad, isBack, keypadPath, currentKeypadPage, keypadSlot, folderPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  // While the profile uses dial functions, the encoder's inspector is the dial functions editor
  // (every encoder binding belongs to a function there); the other modules keep the flat editor.
  const dialFunctionsActive = selection.module === 'm131e' && !!profile.dial_functions;
  const previewRunning = state.preview.status === 'running' || state.keypadPreview.status === 'running';
  const serialMissing = serialScope === 'exact' && !selection.serial;
  const resultFailed = isKeypad ? state.keypadPreview.status === 'failed' : state.preview.status === 'failed';
  const resultDone = isKeypad ? state.keypadPreview.status === 'completed' : state.preview.status === 'completed';
  // Number fields remount when the edited selector changes, discarding typed text.
  const selectorKey = `${selection.module}:${keypadPath.join('/')}:${selection.control}:${selection.gesture}:${selection.serial}`;
  // OBS's current listings for the name fields; asked once per app session, refreshed on demand.
  const catalog = useObsCatalog(catalogStore);
  // Streamlabs' listings for its name fields, the same way.
  const streamlabs = useStreamlabsCatalog(streamlabsStore);
  // Which tab of the dial editor (functions or dial screen) is open; view state only.
  const [dialTab, setDialTab] = useState<'functions' | 'display'>('functions');
  // The physical fader's range and jitter since the last reset, for the hardware notes; view state only.
  const [faderStats, setFaderStats] = useState<FaderStats>(emptyFaderStats);
  useEffect(() => { if (liveFader !== undefined) setFaderStats(stats => recordFader(stats, liveFader)); }, [liveFader]);
  useEffect(() => { setFaderStats(emptyFaderStats); }, [selection.module, selection.control]);
  const declared = profile.variables ?? [];
  return <aside className="inspector" aria-label={t('inspector.aria')}><VariablesContext.Provider value={declared}>
    {selected.supported ? <>
      {isKeypad && <><label htmlFor="keypad-page-name">{keypadDepth(keypadPath) ? t('inspector.folder', { depth: keypadDepth(keypadPath) }) : t('inspector.rootPage')}</label><input id="keypad-page-name" disabled={disabled} value={currentKeypadPage.name} aria-describedby="keypad-page-name-error" onChange={event => session.edit(renameKeypadPage(profile, keypadPath, event.target.value), `page-name:${keypadPath.join('/')}`)} /><LabelError id="keypad-page-name-error" value={currentKeypadPage.name} field="Page name" />{keypadPath.length > 1 && <button className="button compact" disabled={disabled} onClick={editor.deleteCurrentFolder}><Trash2 size={16} />{t('inspector.deleteFolder')}</button>}<div className="inspector-divider" /></>}
      {isKeypad && <><ScreenBrightnessControl screen="keys" label={t('inspector.keyBrightness')} profile={profile} session={session} disabled={disabled} /><div className="inspector-divider" /></>}
      <h2>{isBack ? t('inspector.back', { depth: keypadDepth(keypadPath) }) : controlLabel(selection.module, selection.control)}</h2>
      {!isBack && !folderPage && <div className="inspector-clipboard">
        <button type="button" className="button compact" disabled={disabled || (isKeypad ? !keypadSlot : !binding)} data-shortcut={hint('C', mac)} title={withHint(t('inspector.copyTip'), 'C', mac)} onClick={() => { editor.copySelection(); }}><Copy size={16} />{t('inspector.copy')}</button>
        <button type="button" className="button compact" disabled={disabled || !editor.clipboard} data-shortcut={hint('V', mac)} title={withHint(t('inspector.pasteTip'), 'V', mac)} onClick={() => { editor.pasteSelection(); }}><ClipboardPaste size={16} />{t('inspector.paste')}</button>
      </div>}
      <ControlWarnings profile={profile} editor={editor} selected={{ module: selection.module, control: selection.control, keypadPageId: isKeypad ? currentKeypadPage.id : undefined }}
        excludeKinds={dialFunctionsActive ? ['dial-flat-conflict'] : undefined} />
      {isKeypad && !isBack && <KeyArtworkEditor key={`${state.draftGeneration}:${currentKeypadPage.id}:${selection.control}`} session={session} path={keypadPath} control={selection.control} artwork={currentKeypadPage.artwork?.[String(selection.control)]} action={keypadSlot?.type === 'action' ? keypadSlot.bindings[0]?.action : null} initialText={keypadSlot?.type === 'action' ? keypadSlot.label : ''} disabled={disabled} invoke={invoke} />}
      {isKeypad && <>{isBack ? <p className="fine-print">{t('inspector.backNote')}</p> : folderPage ? <><button className="button" onClick={() => editor.navigateKeypad([...keypadPath, folderPage.id])}><Folder size={17} />{t('inspector.openFolder', { name: folderPage.name })}</button><button className="button" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={17} />{t('inspector.deleteFolder')}</button></> : <>{keypadSlot?.type === 'action' ? <><label htmlFor="keypad-label">{t('inspector.keyLabel')}</label><input id="keypad-label" disabled={disabled} value={keypadSlot.label} aria-describedby="keypad-label-error" onChange={event => session.edit(renameKeypadAction(profile, keypadPath, selection.control, event.target.value), `key-label:${keypadPath.join('/')}:${selection.control}`)} /><LabelError id="keypad-label-error" value={keypadSlot.label} field="Key label" /><button className="button compact" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={16} />{t('inspector.clearKey')}</button></> : <button className="button" disabled={disabled || keypadDepth(keypadPath) >= keypadMaximumDepth || profile.keypad!.pages.length >= keypadMaximumPages} onClick={editor.createFolder}><Folder size={17} />{t('inspector.createFolder')}</button>}</>}</>}
      {dialFunctionsActive ? <DialEditorTabs tab={dialTab} onTab={setDialTab} profile={profile} session={session} state={state} disabled={disabled} invoke={invoke} setPage={setPage} catalog={catalogStore} streamlabs={streamlabsStore} />
      : !(isKeypad && (isBack || folderPage)) && <>
      <label htmlFor="binding-gesture">{t('inspector.gesture')}</label><SelectField id="binding-gesture" disabled={!editorGestures(profile, selection.module).length} value={selection.gesture} onChange={event => editor.changeSelection({ ...selection, gesture: event.target.value as Gesture })}>
        {editorGestures(profile, selection.module).map(gesture => <option key={gesture} value={gesture}>{gestureLabels[gesture]}</option>)}
      </SelectField>
      <div className="inspector-divider" />
      <fieldset disabled={disabled || serialMissing || !editorGestures(profile, selection.module).length}>
        <ActionFields action={binding?.action ?? null} fader={selection.module === 'm005s'} selectorKey={selectorKey} onChange={editor.mutateAction} onSetUpObs={setPage ? () => setPage('settings') : undefined} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />
      </fieldset>
      <div className="inspector-divider" />
      <details className="advanced">
        <summary>{t('inspector.advanced')}</summary>
        <label htmlFor="serial-scope">{t('inspector.limit')}</label><SelectField id="serial-scope" value={serialScope} onChange={event => editor.changeSerialScope(event.target.value as SerialScope)}><option value="any">{t('inspector.anyDevice')}</option><option value="exact">{t('inspector.oneDevice')}</option></SelectField>
        {serialScope === 'exact' && <><label className="spaced-label" htmlFor="serial-number">{t('inspector.serial')}</label><input id="serial-number" value={selection.serial} spellCheck={false} onChange={event => editor.changeSelection({ ...selection, serial: event.target.value })} /><p className="fine-print">{t('inspector.serialNote')}</p></>}
        <p className="fine-print">{t('inspector.serialHint')}</p>
      </details>
      {selection.module === 'm005s' && <div className="position-preview"><label htmlFor="fader-position">{t('inspector.simFader')}</label><div className="position-inputs"><input id="fader-position" type="range" min={0} max={faderMaximum} step={1} value={editor.faderPosition} disabled={disabled || state.preview.status === 'running'} onChange={event => editor.changeFaderPosition(Number(event.target.value))} /><label className="sr-only" htmlFor="fader-position-value">{t('inspector.faderValue')}</label><IntegerField id="fader-position-value" value={editor.faderPosition} minimum={0} maximum={faderMaximum} disabled={disabled || state.preview.status === 'running'} onCommit={editor.changeFaderPosition} /></div>{liveFader !== undefined && <div className="fader-readout" role="status"><strong>{t('inspector.faderNow', { value: liveFader })}</strong> <span>{faderStatsText(faderStats)}</span> <button type="button" className="button" onClick={() => setFaderStats(emptyFaderStats)}>{t('inspector.reset')}</button></div>}<p className="fine-print">{t('inspector.faderScale')}</p></div>}
      <div className="inspector-test">
        <button className="button compact" disabled={disabled || previewRunning || serialMissing} onClick={() => { void editor.testBinding(); }}><Play size={15} />{t(selection.module === 'm005s' ? 'inspector.testPosition' : isKeypad ? 'inspector.simulateKey' : 'inspector.testGesture')}</button>
        <p className={`preview-line${resultDone ? ' done' : ''}`} role={resultFailed ? 'alert' : 'status'}><TestResult t={t} desktop={desktop} preview={state.preview} keypadPreview={state.keypadPreview} isKeypad={isKeypad} /></p>
        <p className="fine-print">{t('inspector.testsNote')}</p>
      </div>
      </>}
      {/* The dial functions editor always lives here: while functions are off it offers to add
          them (flat bindings keep working until then), and with functions on it is the only
          editor, since every encoder binding then belongs to a function. */}
      {!dialFunctionsActive && selection.module === 'm131e' && <DialEditorTabs tab={dialTab} onTab={setDialTab} profile={profile} session={session} state={state} disabled={disabled} invoke={invoke} setPage={setPage} catalog={catalogStore} streamlabs={streamlabsStore} />}
      <div className="inspector-divider" />
      <details className="advanced inspector-variables" open={declared.length > 0 || undefined}>
        <summary>Variables{declared.length > 0 ? ` (${declared.length})` : ''}</summary>
        <VariablesPanel variables={declared} uses={variableUseCounts(profile)} disabled={disabled}
          onChange={(next, field) => session.edit({ ...profile, variables: next.length > 0 ? next : undefined }, field)}
          onRename={(from, to) => session.edit(renameVariable(profile, from, to), `variable-rename:${from}`)} />
      </details>
    </> : <><h2>{selected.name}</h2><div className="status offline"><span className="status-dot" />{t('inspector.notAvailable')}</div><p className="inspector-note">{t('inspector.unsupported')}</p></>}
  </VariablesContext.Provider></aside>;
}

// The dial editor's two tabs. Functions is the dial functions editor (or its
// offer to add them); Display sets what the dial screen shows for this
// profile. The tab choice is view state; it survives nothing.
export function DialEditorTabs({ tab, onTab, profile, session, state, disabled, invoke, setPage, catalog: catalogStore, streamlabs: streamlabsStore }: {
  tab: 'functions' | 'display'; onTab(tab: 'functions' | 'display'): void; profile: Profile; session: ProfileSession; state: SessionState; disabled: boolean; invoke?: Invoke; setPage?: (page: Page) => void; catalog?: ObsCatalogStore; streamlabs?: StreamlabsCatalogStore;
}) {
  const t = useT();
  return <section className="dial-editor" aria-label={t('inspector.dialEditor')}>
    <div role="tablist" aria-label={t('inspector.dialSections')} className="dial-tabs">
      <button type="button" role="tab" id="dial-tab-functions" aria-selected={tab === 'functions'} aria-controls="dial-panel-functions" className={tab === 'functions' ? 'active' : ''} onClick={() => onTab('functions')}>{t('inspector.functions')}</button>
      <button type="button" role="tab" id="dial-tab-display" aria-selected={tab === 'display'} aria-controls="dial-panel-display" className={tab === 'display' ? 'active' : ''} onClick={() => onTab('display')}>{t('inspector.display')}</button>
    </div>
    {tab === 'functions'
      ? <div role="tabpanel" id="dial-panel-functions" aria-labelledby="dial-tab-functions"><DialFunctionsPanel profile={profile} session={session} state={state} disabled={disabled} setPage={setPage} catalog={catalogStore} streamlabs={streamlabsStore} /></div>
      : <div role="tabpanel" id="dial-panel-display" aria-labelledby="dial-tab-display"><DialDisplayPanel profile={profile} session={session} disabled={disabled} invoke={invoke} /></div>}
  </section>;
}

// The one-line result of the inspector's Test button, for a gesture, a fader position or a key click.
export function TestResult({ desktop, preview, keypadPreview, isKeypad, t = english }: { t?: Translate; desktop: boolean; preview: SessionState['preview']; keypadPreview: KeypadPreview; isKeypad: boolean }) {
  if (isKeypad) {
    if (keypadPreview.status === 'idle') return <><CircleHelp size={15} /><span>{t('inspector.keyIdle')}</span></>;
    if (keypadPreview.status === 'running') return <><LoaderCircle size={15} className="spinning" /><span>{t('inspector.resolvingKey')}</span></>;
    if (keypadPreview.status === 'failed') return <><CircleHelp size={15} /><span className="error-text">{keypadPreview.error}</span></>;
    const navigation = keypadPreview.result.events.find(event => event.navigation)?.navigation;
    const actions = keypadPreview.result.events.filter(event => event.action);
    return <><CheckCircle2 size={15} /><span>{navigation ? navigation.depth ? t('inspector.navigatedTo', { depth: navigation.depth }) : t('inspector.navigatedRoot') : actions.length ? actions.map(event => `${gestureLabels[event.gesture]}: would ${actionLabel(event.action).toLowerCase()}`).join('; ') : t('inspector.noKeyAction')}</span></>;
  }
  if (preview.status === 'idle') return <><CircleHelp size={15} /><span>{desktop ? t('inspector.idle') : t('inspector.needDesktop')}</span></>;
  if (preview.status === 'running') return <><LoaderCircle size={15} className="spinning" /><span>{t('inspector.resolving')}</span></>;
  if (preview.status === 'failed') return <><CircleHelp size={15} /><span className="error-text">{preview.error}</span></>;
  return <><CheckCircle2 size={15} /><span>{bindingPreviewText(preview.event)}</span></>;
}
