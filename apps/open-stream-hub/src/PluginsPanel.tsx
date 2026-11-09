import { FolderOpen, RotateCcw } from 'lucide-react';
import { useT } from './i18n-react';
import type { MessageKey } from './locales/en';
import { usePluginListing } from './plugins-context';
import type { ListedPlugin } from './plugins-model';
import type { Invoke } from './profile-session';

const stateKeys: Record<string, MessageKey> = {
  running: 'plugins.state.running', starting: 'plugins.state.starting', restarting: 'plugins.state.restarting',
  stopped: 'plugins.state.stopped', unusable: 'plugins.state.unusable',
};

// Settings > Plugins: every folder in the plugins folder, with what its manifest says it does and needs, and, while a
// Live profile uses it, its state, last error and recent log. A plugin runs as you; the permissions it lists are what
// it says it needs, which nothing enforces, so the panel says so.
export function PluginsPanel({ desktop, invoke }: { desktop: boolean; invoke: Invoke }) {
  const t = useT();
  const { listing, error, refresh } = usePluginListing(desktop ? invoke : undefined);
  return <section className="panel plugins-panel">
    <h2>{t('plugins.title')}</h2>
    <p className="fine-print">{t('plugins.intro')}</p>
    {!desktop ? <p className="fine-print">{t('plugins.notDesktop')}</p> : <>
      {listing && <><label>{t('plugins.folder')}</label><code className="storage-path">{listing.folder}</code></>}
      <div className="settings-actions">
        <button type="button" className="button" onClick={() => { void invoke('open_plugins_folder').catch(() => {}); }}><FolderOpen size={16} />{t('plugins.open')}</button>
        <button type="button" className="button" onClick={refresh}><RotateCcw size={16} />{t('plugins.refresh')}</button>
      </div>
      {error && <p className="field-error" role="alert">{t('plugins.listFailed', { error })}</p>}
      {listing && listing.plugins.length === 0 && <p className="fine-print" role="status">{t('plugins.none')}</p>}
      {listing && listing.plugins.length > 0 && <ul className="plugin-list">{listing.plugins.map(plugin => <PluginItem key={plugin.folder} plugin={plugin} />)}</ul>}
    </>}
  </section>;
}

function PluginItem({ plugin }: { plugin: ListedPlugin }) {
  const t = useT();
  const { manifest, status } = plugin;
  const permissions = manifest?.permissions;
  return <li className="plugin-item">
    <div className="plugin-heading">
      <strong>{manifest ? `${manifest.name} ${manifest.version}` : plugin.folder}</strong>
      {manifest && <code>{manifest.id}</code>}
      <span className="fine-print">{status ? t(stateKeys[status.state] ?? 'plugins.state.unusable') : manifest ? t('plugins.state.idle') : t('plugins.state.unusable')}</span>
    </div>
    {plugin.error && <p className="field-error" role="alert">{plugin.error}</p>}
    {!plugin.error && status?.error && <p className="field-error" role="alert">{status.error}</p>}
    {manifest && <>
      <p className="fine-print">{t('plugins.actions', { actions: manifest.actions.map(action => action.label).join(', ') })}</p>
      <p className="fine-print">{t('plugins.command', { command: manifest.command.join(' ') })}</p>
      <p className="fine-print">{[
        permissions?.network?.length ? t('plugins.permissions.network', { hosts: permissions.network.join(', ') }) : t('plugins.permissions.noNetwork'),
        permissions?.variables?.length ? t('plugins.permissions.variables', { names: permissions.variables.join(', ') }) : t('plugins.permissions.noVariables'),
        permissions?.programs ? t('plugins.permissions.programs') : t('plugins.permissions.noPrograms'),
      ].join(' ')} {t('plugins.permissions.note')}</p>
    </>}
    {status && status.logs.length > 0 && <details className="advanced"><summary>{t('plugins.log', { count: status.logs.length })}</summary>
      <pre className="plugin-log">{status.logs.join('\n')}</pre>
      {status.dropped_logs > 0 && <p className="fine-print">{t('plugins.dropped', { count: status.dropped_logs })}</p>}
    </details>}
  </li>;
}
