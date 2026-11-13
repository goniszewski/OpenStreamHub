import { TriangleAlert } from 'lucide-react';
import { ModalDialog } from './controls';
import { english } from './i18n-react';
import type { Translate } from './i18n-react';
import { isSecretParam } from './plugins-model';
import { codeList, richText } from './rich-text';
import type { PluginManifest, PluginParam, PluginSecretStatus } from './plugins-model';

// The first-enable dialog: everything the plugin's manifest declares, with a plain warning that it runs as the user
// and that its permissions are what it says, not limits (apart from the variables it may set). Enabling approves the
// fingerprint the dialog was opened with, so a plugin.json that changed meanwhile is refused and reviewed again.
// Hook-free and controlled, like the other dialogs.
// One parameter as the dialog names it: its label, type and what it is marked.
function paramText(param: PluginParam, t: Translate): string {
  const flags = [
    param.secret && param.type === 'text' ? t('plugins.dialog.flag.secret') : null,
    param.variables && param.type === 'text' ? t('plugins.dialog.flag.variables') : null,
    param.required ? t('plugins.dialog.flag.required') : null,
  ].filter((flag): flag is string => flag !== null);
  const type = t(`plugins.dialog.type.${param.type}` as const);
  return t('plugins.dialog.param', { label: param.label, type: [type, ...flags].join(', ') });
}

// An action's or condition's parameters, or that it has none.
const params = (list: PluginParam[] | undefined, t: Translate) => list?.length
  ? <ul>{list.map(param => <li key={param.id}>{paramText(param, t)}</li>)}</ul>
  : <span className="fine-print"> ({t('plugins.dialog.noParams')})</span>;

// `stored` are the plugin's secrets as the listing reports them: those already set are named, since enabling sends
// them to this manifest's program.
export function PluginEnableDialog({ open, manifest, folder, stored = [], busy = false, error = null, onCancel, onEnable, t = english }: {
  open: boolean; manifest: PluginManifest; folder: string; stored?: PluginSecretStatus[]; busy?: boolean; error?: string | null;
  onCancel(): void; onEnable(): void; t?: Translate;
}) {
  const storedLabels = stored.filter(secret => secret.set === true).map(secret => secret.label);
  const [program, ...args] = manifest.command;
  const permissions = manifest.permissions;
  const actions = manifest.actions ?? [];
  const conditions = manifest.conditions ?? [];
  // Each secret once by id, named as the first action (then condition) that declares it names it (secret_params in plugins.rs).
  const seen = new Set<string>();
  const secrets = [...actions, ...conditions].flatMap(item => item.params ?? [])
    .filter(param => isSecretParam(param) && !seen.has(param.id) && seen.add(param.id) !== undefined)
    .map(param => param.label);
  return <ModalDialog open={open} role="alertdialog" labelledBy="plugin-enable-title" describedBy="plugin-enable-warning plugin-enable-intro" onCancel={onCancel} className="execute-dialog plugin-enable-dialog">
    <h2 id="plugin-enable-title">{t('plugins.dialog.title', { name: manifest.name })}</h2>
    <p id="plugin-enable-warning" className="execute-warning"><TriangleAlert size={18} aria-hidden="true" /><span>{t('plugins.dialog.warning')}</span></p>
    <p id="plugin-enable-intro">{richText(t('plugins.dialog.intro'))}</p>
    <div className="execute-details" tabIndex={0} role="region" aria-label={t('plugins.dialog.details')}>
      <ul className="execute-capabilities">
        <li>{richText(t('plugins.dialog.id'), { id: <code>{manifest.id}</code> })}</li>
        <li>{t('plugins.dialog.version', { version: manifest.version })}</li>
        <li>{richText(t('plugins.dialog.folder'), { folder: <code>{folder}</code> })}</li>
        <li className="plugin-enable-command">{richText(t('plugins.dialog.program'), { program: <code>{program}</code> })}{' '}
          {args.length ? richText(t('plugins.dialog.arguments'), { arguments: <code>{args.map(arg => JSON.stringify(arg)).join(' ')}</code> }) : t('plugins.dialog.noArguments')}</li>
      </ul>
      {actions.length > 0 && <>
        <h3>{t('plugins.dialog.actions')}</h3>
        <ul className="plugin-enable-actions">{actions.map(action => <li key={action.id}>{action.label}
          {params(action.params, t)}
          {action.events && <p className="plugin-enable-events">{t('plugins.dialog.events')}</p>}
        </li>)}</ul>
      </>}
      {conditions.length > 0 && <>
        <h3>{t('plugins.dialog.conditions')}</h3>
        <p className="plugin-enable-checks">{t('plugins.dialog.conditionsNote')}</p>
        <ul className="plugin-enable-conditions">{conditions.map(condition => <li key={condition.id}>{condition.label}{params(condition.params, t)}</li>)}</ul>
      </>}
      <h3>{t('plugins.dialog.permissions')}</h3>
      <ul className="plugin-enable-permissions">
        <li>{permissions?.network?.length ? richText(t('plugins.permissions.network'), { hosts: codeList(permissions.network) }) : t('plugins.permissions.noNetwork')}</li>
        <li>{permissions?.variables?.length ? richText(t('plugins.permissions.variables'), { names: codeList(permissions.variables) }) : t('plugins.permissions.noVariables')}</li>
        <li>{permissions?.programs ? t('plugins.permissions.programs') : t('plugins.permissions.noPrograms')}</li>
      </ul>
      <p className="plugin-enable-secrets">{secrets.length ? t('plugins.dialog.secrets', { labels: secrets.join(', ') }) : t('plugins.dialog.noSecrets')}</p>
      {storedLabels.length > 0 && <p className="plugin-enable-stored">{t('plugins.dialog.storedSecrets', { labels: storedLabels.join(', ') })}</p>}
    </div>
    {error && <p className="field-error" role="alert">{t('plugins.actionFailed', { error })}</p>}
    <div className="dialog-actions">
      <button type="button" className="button" autoFocus onClick={onCancel}>{t('plugins.dialog.cancel')}</button>
      <button type="button" className="button danger" disabled={busy} onClick={onEnable}>{t('plugins.dialog.enable')}</button>
    </div>
  </ModalDialog>;
}
