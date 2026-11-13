import { useState } from 'react';
import type { ReactNode } from 'react';
import { FolderOpen, PackagePlus, RotateCcw } from 'lucide-react';
import { useT } from './i18n-react';
import type { MessageKey } from './locales/en';
import { ModalDialog } from './controls';
import { PluginEnableDialog, PluginInstallDialog } from './PluginEnableDialog';
import { usePluginListing } from './plugins-context';
import type { InstallReview, ListedPlugin, PluginManifest, PluginSecretStatus } from './plugins-model';
import type { Invoke } from './profile-session';
import { codeList, richText } from './rich-text';

const stateKeys: Record<string, MessageKey> = {
  running: 'plugins.state.running', starting: 'plugins.state.starting', restarting: 'plugins.state.restarting',
  stopped: 'plugins.state.stopped', unusable: 'plugins.state.unusable', not_enabled: 'plugins.state.notEnabled',
};

// The plugin under review in the first-enable dialog, with the fingerprint of the plugin.json the dialog shows.
interface Review { manifest: PluginManifest; fingerprint: string; folder: string; secrets: PluginSecretStatus[] }

// Settings > Plugins: every folder in the plugins folder, with what its manifest says it does and needs, whether it
// is enabled, its secrets and, while a Live profile uses it, its state, last error and recent log. A plugin runs only
// after the user enabled the exact plugin.json the first-enable dialog showed. A plugin runs as you; the permissions
// it lists are what it says it needs, which nothing enforces, so the panel and the dialog say so.
export function PluginsPanel({ desktop, invoke }: { desktop: boolean; invoke: Invoke }) {
  const t = useT();
  const { listing, error, refresh } = usePluginListing(desktop ? invoke : undefined);
  const [review, setReview] = useState<Review | null>(null);
  const [reviewError, setReviewError] = useState<string | null>(null);
  // A refused enable closes the dialog, since what it shows may be stale; the reason stays here.
  const [enableError, setEnableError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const open = (plugin: ListedPlugin) => {
    if (!plugin.manifest || !plugin.fingerprint || !listing) return;
    setReviewError(null);
    setEnableError(null);
    setReview({ manifest: plugin.manifest, fingerprint: plugin.fingerprint, folder: `${listing.folder}/${plugin.folder}`, secrets: plugin.secrets ?? [] });
  };
  const enable = () => {
    if (!review) return;
    setBusy(true);
    setReviewError(null);
    invoke('enable_plugin', { id: review.manifest.id, fingerprint: review.fingerprint })
      .then(() => { setReview(null); refresh(); })
      .catch((reason: unknown) => {
        setEnableError(t('plugins.enableFailed', { name: review.manifest.name, error: String(reason) }));
        setReview(null);
        refresh();
      })
      .finally(() => setBusy(false));
  };
  // Install…: the folder picker (null when cancelled), then the review of the chosen folder; a refused review is shown
  // here. Installing closes the dialog either way, since a refusal (plugin.json changed) makes what it shows stale.
  const [install, setInstall] = useState<InstallReview | null>(null);
  const [manageError, setManageError] = useState<string | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [removing, setRemoving] = useState<ListedPlugin | null>(null);
  // The plugin being removed right now (stopping it can take a few seconds); its Remove… is disabled meanwhile.
  const [removingNow, setRemovingNow] = useState<string | null>(null);
  const chooseInstall = () => {
    setChoosing(true);
    setManageError(null);
    setEnableError(null);
    invoke<string | null>('choose_plugin_folder')
      .then(path => path === null ? null : invoke<InstallReview>('review_plugin_install', { path }).then(setInstall))
      .catch((reason: unknown) => setManageError(t('plugins.installFailed', { error: String(reason) })))
      .finally(() => setChoosing(false));
  };
  const runInstall = (enable: boolean) => {
    if (!install?.plugin.fingerprint || busy) return;
    setBusy(true);
    // A failure after the copy was swapped in (such as enabling it) comes as "installed: <reason>": it is installed.
    invoke('install_plugin', { path: install.path, fingerprint: install.plugin.fingerprint, enable })
      .catch((reason: unknown) => {
        const error = String(reason);
        setManageError(error.startsWith('installed: ')
          ? t('plugins.installedBut', { error: error.slice('installed: '.length) }) : t('plugins.installFailed', { error }));
      })
      .finally(() => { setInstall(null); setBusy(false); refresh(); });
  };
  const remove = () => {
    if (!removing) return;
    const plugin = removing;
    setRemoving(null);
    setManageError(null);
    setRemovingNow(plugin.folder);
    invoke('remove_plugin', { id: plugin.folder })
      // A failure after the folder was deleted (such as the Keychain keeping a secret) comes as "removed: <reason>".
      .catch((reason: unknown) => {
        const name = plugin.manifest?.name ?? plugin.folder;
        const error = String(reason);
        setManageError(error.startsWith('removed: ')
          ? t('plugins.removedBut', { name, error: error.slice('removed: '.length) }) : t('plugins.removeFailed', { name, error }));
      })
      .finally(() => { setRemovingNow(null); refresh(); });
  };
  const enabled = listing?.plugins.filter(plugin => plugin.approval === 'enabled') ?? [];
  const disabled = listing?.plugins.filter(plugin => plugin.approval !== 'enabled') ?? [];
  const toReview = disabled.filter(plugin => plugin.approval === 'changed').length;
  const items = (plugins: ListedPlugin[]) => <ul className="plugin-list">{plugins.map(plugin =>
    <PluginItem key={plugin.folder} plugin={plugin} invoke={invoke} refresh={refresh} onReview={() => open(plugin)} removeDisabled={removingNow !== null}
      onRemove={() => { setManageError(null); setRemoving(plugin); }} />)}</ul>;
  return <section className="panel plugins-panel">
    <h2>{t('plugins.title')}</h2>
    <p className="fine-print">{richText(t('plugins.intro'))}</p>
    {!desktop ? <p className="fine-print">{t('plugins.notDesktop')}</p> : <>
      {listing && <><label>{t('plugins.folder')}</label><code className="storage-path">{listing.folder}</code></>}
      <div className="settings-actions">
        <button type="button" className="button" disabled={choosing || busy} onClick={chooseInstall}><PackagePlus size={16} />{t('plugins.installButton')}</button>
        <button type="button" className="button" onClick={() => { void invoke('open_plugins_folder').catch(() => {}); }}><FolderOpen size={16} />{t('plugins.open')}</button>
        <button type="button" className="button" onClick={refresh}><RotateCcw size={16} />{t('plugins.refresh')}</button>
      </div>
      {error && <p className="field-error" role="alert">{t('plugins.listFailed', { error })}</p>}
      {enableError && <p className="field-error plugin-enable-error" role="alert">{enableError}</p>}
      {manageError && <p className="field-error plugin-manage-error" role="alert">{manageError}</p>}
      {listing && listing.plugins.length === 0 && <p className="fine-print" role="status">{t('plugins.none')}</p>}
      {listing && listing.plugins.length > 0 && <>
        <PluginGroup id="enabled" initiallyOpen title={t('plugins.group.enabled', { count: enabled.length })}>
          {enabled.length ? items(enabled) : <p className="fine-print">{t('plugins.group.noneEnabled')}</p>}
        </PluginGroup>
        <PluginGroup id="disabled" initiallyOpen={false} title={t('plugins.group.disabled', { count: disabled.length })}
          note={toReview > 0 ? t('plugins.group.toReview', { count: toReview }) : undefined}>
          {disabled.length ? items(disabled) : <p className="fine-print">{t('plugins.group.noneDisabled')}</p>}
        </PluginGroup>
      </>}
    </>}
    {review && <PluginEnableDialog open manifest={review.manifest} folder={review.folder} stored={review.secrets} busy={busy} error={reviewError}
      onCancel={() => setReview(null)} onEnable={enable} t={t} />}
    {install?.plugin.manifest && <PluginInstallDialog review={install} folder={listing ? `${listing.folder}/${install.plugin.folder}` : install.plugin.folder} busy={busy}
      onCancel={() => setInstall(null)} onInstall={runInstall} t={t} />}
    {removing && <ModalDialog open role="alertdialog" labelledBy="plugin-remove-title" describedBy="plugin-remove-message" onCancel={() => setRemoving(null)}>
      <h2 id="plugin-remove-title">{t('plugins.remove.title', { name: removing.manifest?.name ?? removing.folder })}</h2>
      <p id="plugin-remove-message">{t('plugins.remove.message')}</p>
      <div className="dialog-actions">
        <button type="button" className="button" autoFocus onClick={() => setRemoving(null)}>{t('plugins.dialog.cancel')}</button>
        <button type="button" className="button danger" onClick={remove}>{t('plugins.remove.confirm')}</button>
      </div>
    </ModalDialog>}
  </section>;
}

// A collapsible group of plugins. Its contents stay in the page while closed, so a plugin's state is still found.
function PluginGroup({ id, title, note, initiallyOpen, children }: { id: string; title: string; note?: string; initiallyOpen: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(initiallyOpen);
  return <details className="plugin-group" data-group={id} open={open} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary><span>{title}</span>{note && <span className="plugin-group-note">{note}</span>}</summary>
    {children}
  </details>;
}

function PluginItem({ plugin, invoke, refresh, onReview, onRemove, removeDisabled = false }: {
  plugin: ListedPlugin; invoke: Invoke; refresh(): void; onReview(): void; onRemove(): void; removeDisabled?: boolean;
}) {
  const t = useT();
  const [error, setError] = useState<string | null>(null);
  const { manifest, status, approval } = plugin;
  const permissions = manifest?.permissions;
  const run = (command: string, args: Record<string, unknown>) => {
    setError(null);
    return invoke(command, args).then(refresh).catch((reason: unknown) => { setError(String(reason)); throw reason; });
  };
  const stateText = status ? t(stateKeys[status.state] ?? 'plugins.state.unusable')
    : !manifest ? t('plugins.state.unusable') : approval === 'enabled' ? t('plugins.state.idle') : t('plugins.state.notEnabled');
  const state = status?.state ?? (!manifest ? 'unusable' : approval === 'enabled' ? 'idle' : 'not_enabled');
  return <li className="plugin-item" data-state={state}>
    <div className="plugin-heading">
      <div className="plugin-title">
        <strong>{manifest ? manifest.name : plugin.folder}</strong>
        {manifest && <span className="plugin-version">{manifest.version}</span>}
        {manifest && <code className="plugin-id">{manifest.id}</code>}
      </div>
      <span className="plugin-state">{stateText}</span>
    </div>
    {plugin.error && <p className="field-error" role="alert">{plugin.error}</p>}
    {!plugin.error && status?.error && status.state !== 'not_enabled' && <p className="field-error" role="alert">{status.error}</p>}
    {manifest && approval && <div className="plugin-approval">
      {approval === 'enabled' && <><span className="fine-print">{t('plugins.enabled')}</span>
        <button type="button" className="button compact" onClick={() => { void run('disable_plugin', { id: manifest.id }).catch(() => {}); }}>{t('plugins.disable')}</button></>}
      {approval === 'not_enabled' && <><span className="fine-print">{t('plugins.notEnabled')}</span>
        <button type="button" className="button compact" onClick={onReview}>{t('plugins.enable')}</button></>}
      {approval === 'changed' && <><span className="field-error" role="alert">{richText(t('plugins.changed'))}</span>
        <button type="button" className="button compact" onClick={onReview}>{t('plugins.review')}</button></>}
    </div>}
    {error && <p className="field-error" role="alert">{t('plugins.actionFailed', { error })}</p>}
    {manifest && <>
      <dl className="plugin-facts">
        {manifest.actions?.length ? <><dt>{t('plugins.actions')}</dt><dd>{manifest.actions.map(action => action.label).join(', ')}</dd></> : null}
        {manifest.conditions?.length ? <><dt>{t('plugins.conditions')}</dt><dd>{manifest.conditions.map(condition => condition.label).join(', ')}</dd></> : null}
        <dt>{t('plugins.command')}</dt><dd><code>{manifest.command.join(' ')}</code></dd>
        <dt>{t('plugins.declares')}</dt><dd>
          {permissions?.network?.length ? richText(t('plugins.permissions.network'), { hosts: codeList(permissions.network) }) : t('plugins.permissions.noNetwork')}{' '}
          {permissions?.variables?.length ? richText(t('plugins.permissions.variables'), { names: codeList(permissions.variables) }) : t('plugins.permissions.noVariables')}{' '}
          {permissions?.programs ? t('plugins.permissions.programs') : t('plugins.permissions.noPrograms')}
        </dd>
      </dl>
      <p className="fine-print">{t('plugins.permissions.note')}</p>
    </>}
    {manifest && (plugin.secrets?.length ?? 0) > 0 && <div className="plugin-secrets">
      <label>{t('plugins.secrets')}</label>
      <p className="fine-print">{t('plugins.secretsNote')}</p>
      <ul>{plugin.secrets!.map(secret => <SecretRow key={secret.id} plugin={manifest.id} secret={secret} run={run} />)}</ul>
    </div>}
    <div className="plugin-manage">
      <button type="button" className="button compact" disabled={removeDisabled} onClick={onRemove}>{t('plugins.remove')}</button>
    </div>
    {status && status.logs.length > 0 && <details className="advanced"><summary>{t('plugins.log', { count: status.logs.length })}</summary>
      <pre className="plugin-log">{status.logs.join('\n')}</pre>
      {status.dropped_logs > 0 && <p className="fine-print">{t('plugins.dropped', { count: status.dropped_logs })}</p>}
    </details>}
  </li>;
}

// One secret: whether it is set, and Set… / Change… with a password field, and Forget. The value goes to the Keychain
// and never comes back from the backend.
function SecretRow({ plugin, secret, run }: { plugin: string; secret: PluginSecretStatus; run(command: string, args: Record<string, unknown>): Promise<unknown> }) {
  const t = useT();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState('');
  const id = `plugin-secret-${plugin}-${secret.id}`.replace(/[^a-zA-Z0-9_-]/g, '-');
  const save = () => {
    void run('set_plugin_secret', { plugin, param: secret.id, value })
      .then(() => { setValue(''); setEditing(false); })
      .catch(() => {});
  };
  return <li className="plugin-secret" data-secret={secret.id}>
    <strong>{secret.label}</strong>{' '}<span className="fine-print">{t(secret.set === null ? 'plugins.secretUnknown' : secret.set ? 'plugins.secretSet' : 'plugins.secretNotSet')}</span>{' '}
    {!editing && <button type="button" className="button compact" onClick={() => setEditing(true)}>{t(secret.set ? 'plugins.secretChange' : 'plugins.secretSetButton')}</button>}
    {secret.set !== false && <button type="button" className="button compact" onClick={() => { void run('forget_plugin_secret', { plugin, param: secret.id }).catch(() => {}); }}>{t('plugins.secretForget')}</button>}
    {editing && <form className="obs-password-form" onSubmit={event => { event.preventDefault(); save(); }}>
      <label htmlFor={id} className="sr-only">{t('plugins.secretValue', { label: secret.label })}</label>
      <input id={id} type="password" autoComplete="off" spellCheck={false} value={value} onChange={event => setValue(event.target.value)} />
      <button type="submit" className="button compact" disabled={value === ''}>{t('plugins.secretSave')}</button>
      <button type="button" className="button compact" onClick={() => { setValue(''); setEditing(false); }}>{t('plugins.secretCancel')}</button>
    </form>}
  </li>;
}
