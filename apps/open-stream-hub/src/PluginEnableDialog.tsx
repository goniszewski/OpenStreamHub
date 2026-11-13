import type { ReactNode } from 'react';
import { TriangleAlert } from 'lucide-react';
import { ModalDialog } from './controls';
import { english } from './i18n-react';
import type { Translate } from './i18n-react';
import { formatSize, isSecretParam } from './plugins-model';
import { codeList, richText } from './rich-text';
import type { InstallReview, PluginManifest, PluginParam, PluginSecretStatus } from './plugins-model';

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

// What a plugin's manifest declares, as the first-enable and install dialogs show it: the warning that it runs as you,
// then (in a scrolling region) `facts` first, its id, version, folder, program and arguments, actions, conditions,
// permissions and secrets. `stored` are the plugin's secrets as the listing reports them: those already set are named,
// since enabling sends them to this manifest's program.
export function PluginReviewBody({ manifest, folder, stored = [], warningId, intro, introId, facts, t }: {
  manifest: PluginManifest; folder: string; stored?: PluginSecretStatus[]; warningId: string; intro: string; introId: string;
  facts?: ReactNode; t: Translate;
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
  return <>
    <p id={warningId} className="execute-warning"><TriangleAlert size={18} aria-hidden="true" /><span>{t('plugins.dialog.warning')}</span></p>
    <p id={introId}>{richText(intro)}</p>
    <div className="execute-details" tabIndex={0} role="region" aria-label={t('plugins.dialog.details')}>
      {facts}
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
  </>;
}

// The first-enable dialog: everything the plugin's manifest declares, with a plain warning that it runs as the user
// and that its permissions are what it says, not limits (apart from the variables it may set). Enabling approves the
// fingerprint the dialog was opened with, so a plugin.json that changed meanwhile is refused and reviewed again.
// Hook-free and controlled, like the other dialogs.
export function PluginEnableDialog({ open, manifest, folder, stored = [], busy = false, error = null, onCancel, onEnable, t = english }: {
  open: boolean; manifest: PluginManifest; folder: string; stored?: PluginSecretStatus[]; busy?: boolean; error?: string | null;
  onCancel(): void; onEnable(): void; t?: Translate;
}) {
  return <ModalDialog open={open} role="alertdialog" labelledBy="plugin-enable-title" describedBy="plugin-enable-warning plugin-enable-intro" onCancel={onCancel} className="execute-dialog plugin-enable-dialog">
    <h2 id="plugin-enable-title">{t('plugins.dialog.title', { name: manifest.name })}</h2>
    <PluginReviewBody manifest={manifest} folder={folder} stored={stored} warningId="plugin-enable-warning" introId="plugin-enable-intro"
      intro={t('plugins.dialog.intro')} t={t} />
    {error && <p className="field-error" role="alert">{t('plugins.actionFailed', { error })}</p>}
    <div className="dialog-actions">
      <button type="button" className="button" autoFocus onClick={onCancel}>{t('plugins.dialog.cancel')}</button>
      <button type="button" className="button danger" disabled={busy} onClick={onEnable}>{t('plugins.dialog.enable')}</button>
    </div>
  </ModalDialog>;
}

// The install review: the same review of the chosen folder's plugin.json, plus what installing does: where it comes
// from and how much it copies, and, when that id is installed already, what it replaces, whether the new version is
// older, and whether it is enabled after a plain Install (only when this exact plugin.json is approved already).
// Install and enable also approves the fingerprint shown, which the app checks again against the copy.
export function PluginInstallDialog({ review, folder, busy = false, onCancel, onInstall, t = english }: {
  review: InstallReview; folder: string; busy?: boolean; onCancel(): void; onInstall(enable: boolean): void; t?: Translate;
}) {
  const manifest = review.plugin.manifest!;
  const installed = review.installed;
  const facts = <ul className="execute-capabilities plugin-install-facts">
    <li>{richText(t('plugins.install.source'), { path: <code>{review.path}</code> })}</li>
    <li>{t('plugins.install.size', { count: review.files, size: formatSize(review.bytes) })}</li>
    {installed && <li className="plugin-install-replaces">{installed.version !== null
      ? t('plugins.install.replaces', { version: installed.version }) : t('plugins.install.replacesUnreadable')}</li>}
    {installed?.version != null && review.older && <li className="field-error plugin-install-older">
      {t('plugins.install.older', { version: manifest.version, installed: installed.version })}</li>}
    {/* review.plugin.approval is what plain Install leaves: enabled only when this exact plugin.json is approved now. */}
    {review.plugin.approval === 'enabled'
      ? <li className="plugin-install-approval">{richText(t('plugins.install.staysEnabled'))}</li>
      : (installed?.approval === 'enabled' || installed?.approval === 'changed')
        && <li className="field-error plugin-install-approval">{richText(t('plugins.install.reenable'))}</li>}
  </ul>;
  // While installing, the dialog stays: closing it would not stop the install, and its result lands in the panel.
  return <ModalDialog open role="alertdialog" labelledBy="plugin-install-title" describedBy="plugin-install-warning plugin-install-intro" onCancel={() => { if (!busy) onCancel(); }} className="execute-dialog plugin-enable-dialog plugin-install-dialog">
    <h2 id="plugin-install-title">{t('plugins.install.title', { name: manifest.name })}</h2>
    <PluginReviewBody manifest={manifest} folder={folder} stored={review.plugin.secrets} warningId="plugin-install-warning" introId="plugin-install-intro"
      intro={t('plugins.install.intro')} facts={facts} t={t} />
    <div className="dialog-actions wrap">
      <button type="button" className="button" autoFocus disabled={busy} onClick={onCancel}>{t('plugins.dialog.cancel')}</button>
      <button type="button" className="button" disabled={busy} onClick={() => onInstall(false)}>{t('plugins.install.install')}</button>
      <button type="button" className="button danger" disabled={busy} onClick={() => onInstall(true)}>{t('plugins.install.installAndEnable')}</button>
    </div>
  </ModalDialog>;
}
