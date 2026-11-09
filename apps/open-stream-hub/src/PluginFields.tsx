import { useEffect, useState } from 'react';
import { FolderOpen, RotateCcw } from 'lucide-react';
import type { PluginAction, PluginValue, StepAction } from './contracts';
import { SelectField } from './controls';
import { useT } from './i18n-react';
import { pluginIssue } from './action-model';
import { isSecretParam, pluginActionIssues, pluginActionWith, pluginParamIssue, pluginSecretSet, resolvePluginAction, usablePlugins, withPluginParam } from './plugins-model';
import type { PluginParam } from './plugins-model';
import { usePlugins } from './plugins-context';
import { useVariables } from './variables-context';
import type { Invoke } from './profile-session';

// A plugin action: the plugin and one of its actions, chosen from the installed manifests, and a form the manifest's
// parameter types generate. No plugin-supplied HTML. A plugin that is missing, unusable or too old is said inline and
// the key keeps its settings, so installing or updating the plugin makes it work again. A secret parameter has no
// input: its value is in the Keychain, set in Settings > Plugins.
export function PluginFields({ prefix, action, onChange, invoke, hint }: {
  prefix: string; action: PluginAction; onChange(action: StepAction, field?: string): void; invoke?: Invoke;
  hint: (summary: string, more: string) => React.ReactNode;
}) {
  const t = useT();
  const id = (name: string) => `${prefix}${name}`;
  const { listing, error, refresh } = usePlugins();
  const installed = usablePlugins(listing);
  const declared = useVariables().map(variable => variable.name);
  const resolved = resolvePluginAction(listing, action);
  // A fresh pick has no plugin yet: take the first installed one, so the form shows at once.
  const first = installed[0];
  useEffect(() => {
    if (action.plugin === '' && first) onChange(pluginActionWith(first, first.actions[0].id), 'plugin-id');
  }, [action.plugin, first, onChange]);

  const openFolder = invoke ? () => { void invoke('open_plugins_folder').catch(() => {}); } : undefined;
  const folderNote = <p className="fine-print">
    {listing ? t('actions.plugin.folder', { folder: listing.folder }) : error ? t('actions.plugin.listFailed', { error }) : t('actions.plugin.notDesktop')}
    {' '}{openFolder && <button type="button" className="button compact" onClick={openFolder}><FolderOpen size={16} />{t('actions.plugin.openFolder')}</button>}
    {invoke && <button type="button" className="button compact" onClick={refresh}><RotateCcw size={16} />{t('actions.plugin.refresh')}</button>}
  </p>;

  const shapeIssue = action.plugin === '' ? null : pluginIssue(action);
  const issues = resolved.declared ? pluginActionIssues(resolved.declared, action, declared) : [];
  const extra = issues.filter(issue => issue.startsWith('The plugin has no parameter'));
  return <div className="action-fields plugin-fields">
    <label htmlFor={id('plugin-id')}>{t('actions.plugin.plugin')}</label>
    {installed.length === 0 && action.plugin === ''
      ? <p className="fine-print" role="status">{listing ? t('actions.plugin.none') : t('actions.plugin.loading')}</p>
      : <SelectField id={id('plugin-id')} value={action.plugin}
        aria-describedby={resolved.problem ? id('plugin-problem') : undefined}
        onChange={event => {
          const manifest = installed.find(item => item.id === event.target.value);
          if (manifest) onChange(pluginActionWith(manifest, manifest.actions[0].id), 'plugin-id');
        }}>
        {installed.map(manifest => <option key={manifest.id} value={manifest.id}>{`${manifest.name} ${manifest.version}`}</option>)}
        {action.plugin !== '' && !installed.some(manifest => manifest.id === action.plugin) &&
          <option value={action.plugin}>{t('actions.plugin.missingOption', { id: action.plugin })}</option>}
      </SelectField>}
    {resolved.manifest && <>
      <label className="spaced-label" htmlFor={id('plugin-action')}>{t('actions.plugin.action')}</label>
      <SelectField id={id('plugin-action')} value={action.action}
        onChange={event => onChange(pluginActionWith(resolved.manifest!, String(event.target.value), action), 'plugin-action')}>
        {resolved.manifest.actions.map(item => <option key={item.id} value={item.id}>{item.label}</option>)}
        {!resolved.declared && <option value={action.action}>{t('actions.plugin.missingOption', { id: action.action })}</option>}
      </SelectField>
    </>}
    {resolved.problem && <p id={id('plugin-problem')} className="field-error" role="alert">{resolved.problem}</p>}
    {!resolved.problem && shapeIssue && <p className="field-error" role="alert">{shapeIssue}</p>}
    {resolved.declared?.params?.map(param => isSecretParam(param)
      ? <SecretParamNote key={`${action.plugin}:${action.action}:${param.id}`} id={id(`plugin-param-${param.id}`)} param={param}
        plugin={action.plugin} set={pluginSecretSet(listing, action.plugin, param.id)} value={action.params?.[param.id]} invoke={invoke}
        onDrop={() => onChange(withPluginParam(action, param, undefined), `plugin-param-${param.id}`)} onStored={refresh} />
      : <PluginParamField key={`${action.plugin}:${action.action}:${param.id}`} id={id(`plugin-param-${param.id}`)} param={param}
        value={action.params?.[param.id]} issue={pluginParamIssue(param, action.params?.[param.id], declared)}
        onChange={value => onChange(withPluginParam(action, param, value), `plugin-param-${param.id}`)} />)}
    {extra.map(issue => <p key={issue} className="field-error" role="alert">{issue}</p>)}
    {folderNote}
    {hint(t('actions.plugin.hint'), t('actions.plugin.more'))}
  </div>;
}

// A secret: whether the Keychain holds it and where to set it. A value the profile still carries (saved before secrets
// moved to the Keychain) is offered a move: dropped from the draft, which the user saves, and stored in the Keychain.
// The draft is changed at the click, never when the Keychain answers: by then `onDrop` would apply an older draft and
// undo whatever was edited meanwhile. A failed store says so, and Undo brings the value back into the draft.
function SecretParamNote({ id, param, plugin, set, value, invoke, onDrop, onStored }: {
  id: string; param: PluginParam; plugin: string; set: boolean | null | undefined; value: PluginValue | undefined; invoke?: Invoke;
  onDrop(): void; onStored(): void;
}) {
  const t = useT();
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<'moving' | 'moved' | 'dropped' | null>(null);
  // The Keychain already holds a value: moving would replace it, so that takes a second, explicit step.
  const [confirming, setConfirming] = useState(false);
  const status = set === undefined ? '' : ` ${t(set === null ? 'actions.plugin.secretUnknown' : set ? 'actions.plugin.secretSet' : 'actions.plugin.secretNotSet')}`;
  const movable = !!invoke && typeof value === 'string';
  const move = () => {
    if (typeof value !== 'string' || !invoke) return;
    setError(null);
    setDone('moving');
    onDrop();
    invoke('set_plugin_secret', { plugin, param: param.id, value })
      .then(() => { setDone('moved'); onStored(); })
      .catch((reason: unknown) => { setDone(null); setError(String(reason)); });
  };
  const drop = () => { setError(null); setDone('dropped'); onDrop(); };
  return <>
    <label className="spaced-label" htmlFor={id}>{param.label}</label>
    <p id={id} className="fine-print" role="note">{t('actions.plugin.secret')}{status}</p>
    {value !== undefined && <div className="field-error" role="alert">
      <p>{t('actions.plugin.secretInProfile', { label: param.label })}</p>
      {set === true ? <>
        <button type="button" className="button compact primary" onClick={drop}>{t('actions.plugin.secretDrop')}</button>
        {movable && !confirming && <button type="button" className="button compact" onClick={() => setConfirming(true)}>{t('actions.plugin.secretReplace')}</button>}
        {movable && confirming && <p>{t('actions.plugin.secretReplaceConfirm', { label: param.label })}{' '}
          <button type="button" className="button compact danger" onClick={move}>{t('actions.plugin.secretReplaceYes')}</button>{' '}
          <button type="button" className="button compact" onClick={() => setConfirming(false)}>{t('actions.plugin.secretReplaceNo')}</button></p>}
      </> : <>
        {movable && <button type="button" className="button compact" onClick={move}>{t('actions.plugin.secretMove')}</button>}
        <button type="button" className="button compact" onClick={drop}>{t('actions.plugin.secretDrop')}</button>
      </>}
    </div>}
    {done && value === undefined && <p className="fine-print" role="status">{t(done === 'moving' ? 'actions.plugin.secretMoving' : done === 'moved' ? 'actions.plugin.secretMoved' : 'actions.plugin.secretDropped')}</p>}
    {error && <p className="field-error" role="alert">{t('actions.plugin.secretMoveFailed', { error })}</p>}
  </>;
}

function PluginParamField({ id, param, value, issue, onChange }: {
  id: string; param: PluginParam; value: PluginValue | undefined; issue: string | null; onChange(value: PluginValue | undefined): void;
}) {
  const t = useT();
  const described = issue ? `${id}-error` : param.variables ? `${id}-note` : undefined;
  const label = `${param.label}${param.required ? '' : ` ${t('actions.plugin.optional')}`}`;
  const error = issue && <p id={`${id}-error`} className="field-error" role="alert">{issue}</p>;
  switch (param.type) {
    case 'bool':
      return <><label className="setting-toggle spaced-label"><input id={id} type="checkbox" checked={value === true}
        onChange={event => onChange(event.target.checked)} />{param.label}</label>{error}</>;
    case 'choice':
      return <>
        <label className="spaced-label" htmlFor={id}>{label}</label>
        <SelectField id={id} value={typeof value === 'string' ? value : ''} aria-describedby={described}
          onChange={event => onChange(event.target.value === '' ? undefined : String(event.target.value))}>
          {!param.required && <option value="">{t('actions.plugin.notSet')}</option>}
          {param.required && typeof value !== 'string' && <option value="">{t('actions.plugin.choose')}</option>}
          {(param.choices ?? []).map(choice => <option key={choice.id} value={choice.id}>{choice.label}</option>)}
        </SelectField>
        {error}
      </>;
    case 'number':
      return <>
        <label className="spaced-label" htmlFor={id}>{label}</label>
        <input id={id} type="number" inputMode="decimal" min={param.min} max={param.max} value={typeof value === 'number' ? value : ''}
          aria-invalid={!!issue} aria-describedby={described}
          onChange={event => {
            const text = event.target.value.trim();
            const number = Number(text);
            onChange(text === '' ? undefined : Number.isFinite(number) ? number : undefined);
          }} />
        {error}
      </>;
    case 'text':
      return <>
        <label className="spaced-label" htmlFor={id}>{label}</label>
        <input id={id} type="text" spellCheck={false} autoCapitalize="off" autoCorrect="off"
          value={typeof value === 'string' ? value : ''} aria-invalid={!!issue} aria-describedby={described}
          onChange={event => onChange(event.target.value === '' && !param.required ? undefined : event.target.value)} />
        {error}
        {param.variables && <p id={`${id}-note`} className="fine-print">{t('actions.plugin.variables')}</p>}
      </>;
  }
}
