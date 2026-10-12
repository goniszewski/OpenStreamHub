import type { Profile, ServiceModule } from './contracts';
import { ModalDialog } from './controls';
import { packModules } from './preset-packs';
import type { PresetPack, RejectedPack } from './preset-packs';
import { profileModules, starters, startersForModule } from './starter-model';
import { modules } from './ui-model';
import { english } from './i18n-react';
import type { Translate } from './i18n-react';

export type StarterEntry = 'first-run' | 'manager' | 'module';

const headingKey = { 'first-run': 'starter.firstRun', manager: 'starter.manager', module: 'starter.module' } as const;
const introKey = { 'first-run': 'starter.firstRunIntro', manager: 'starter.managerIntro', module: 'starter.moduleIntro' } as const;
const closeKey = { 'first-run': 'starter.empty', manager: 'starter.cancel', module: 'starter.notNow' } as const;

// The starter chooser. One dialog serves all three entries: the first run,
// the profile manager, and a module that was just connected (then only
// starters that use that module are offered). Hook-free and controlled, so
// tests render it as a function and invoke the handlers.
export function StarterDialog({ open, entry, module, onPick, onClose, onGuide, packs = [], rejected = [], packReport = null, onPickPack = () => {}, onLoadPack, t = english }: {
  open: boolean; entry: StarterEntry; module: ServiceModule | null;
  onPick(profile: Profile): void; onClose(): void; onGuide(): void;
  // App preset packs: the valid bundled ones, the bundled files that were refused, the report for a pack file the user
  // picked, and the handlers for picking a pack and loading a file.
  packs?: PresetPack[]; rejected?: RejectedPack[]; packReport?: RejectedPack | null;
  onPickPack?(pack: PresetPack): void; onLoadPack?(file: File): void; t?: Translate;
}) {
  const offered = module ? startersForModule(module) : starters;
  const offeredPacks = module ? packs.filter(pack => packModules(pack).includes(module)) : packs;
  const moduleName = modules.find(item => item.id === module)?.shortName;
  return <ModalDialog open={open} labelledBy="starter-title" describedBy="starter-body" onCancel={onClose} className="starter-dialog">
    <h2 id="starter-title">{entry === 'module' ? t('starter.connected', { name: moduleName ?? '' }) : t(headingKey[entry])}</h2>
    <p id="starter-body">{t(introKey[entry])}</p>
    <div className="starter-cards">
      {offered.map(item => <button key={item.id} type="button" className="starter-card" data-starter={item.id} onClick={() => onPick(item.profile)}>
        <strong>{item.name}</strong>
        <span>{item.description}</span>
        <small>{t('starter.setsUp')} {profileModules(item.profile).map(id => modules.find(candidate => candidate.id === id)?.shortName ?? id).join(', ')}</small>
      </button>)}
    </div>
    {(offeredPacks.length > 0 || onLoadPack || packReport) && <section className="pack-section" aria-labelledby="pack-heading">
      <h3 id="pack-heading">{t('starter.presets')}</h3>
      <p className="fine-print">{t('starter.presetsNote')}</p>
      <div className="starter-cards">
        {offeredPacks.map(pack => <button key={pack.id} type="button" className="starter-card" data-pack={pack.id} title={t('starter.source', { source: pack.source })} onClick={() => onPickPack(pack)}>
          <strong>{pack.name}</strong>
          <span>{pack.description}</span>
          <small>{t('starter.setsUp')} {packModules(pack).map(id => modules.find(candidate => candidate.id === id)?.shortName ?? id).join(', ')} · {t('starter.licence', { licence: pack.license })} · {t(pack.verified ? 'starter.checked' : 'starter.unchecked')}</small>
        </button>)}
      </div>
      {packReport && <div className="service-failure" role="alert"><strong>{t('starter.notLoaded', { file: packReport.file })}</strong>
        <ul>{packReport.issues.map(issue => <li key={issue}>{issue}</li>)}</ul></div>}
      {onLoadPack && <label className="button compact pack-file">{t('starter.loadFile')}
        <input type="file" accept=".json,application/json" className="sr-only" aria-label={t('starter.loadFileLabel')}
          onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; if (file) onLoadPack(file); }} /></label>}
      {rejected.length > 0 && <details className="pack-rejected"><summary>{t('starter.rejected', { count: rejected.length })}</summary>
        {rejected.map(item => <div key={item.file}><strong>{item.file}</strong><ul>{item.issues.map(issue => <li key={issue}>{issue}</li>)}</ul></div>)}</details>}
    </section>}
    <p className="fine-print">{t('starter.safety')}</p>
    <div className="dialog-actions">
      {entry === 'first-run' && <button type="button" className="button starter-guide" onClick={onGuide}>{t('starter.guide')}</button>}
      <span className="spacer" />
      <button type="button" className="button" autoFocus onClick={onClose}>{t(closeKey[entry])}</button>
    </div>
  </ModalDialog>;
}
