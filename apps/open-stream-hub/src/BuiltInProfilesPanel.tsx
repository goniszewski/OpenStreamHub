import type { ProfileSummary } from './contracts';
import { hiddenBuiltInsStore, useHiddenBuiltIns } from './builtin-profiles';
import { starters } from './starter-model';

// Settings: which built-in profiles the profile pickers offer.
export function BuiltInProfilesPanel({ saved, hidden = new Set() }: { saved: ProfileSummary[]; hidden?: ReadonlySet<string> }) {
  return <section className="panel built-in-profiles">
    <h2>Built-in profiles</h2>
    <p className="fine-print">Ready-made profiles that come with the app. Those you show are listed in the profile pickers; choosing one that you do not have yet saves it as a profile of your own, which you can then change. Hiding one only removes it from the pickers: a saved copy stays in the profile manager and keeps running where it runs.</p>
    {starters.map(template => <div key={template.id} className="built-in-profile">
      <label className="setting-toggle">
        <input type="checkbox" checked={!hidden.has(template.id)} onChange={event => hiddenBuiltInsStore.setShown(template.id, event.target.checked)} />
        Show {template.name} in the profile pickers
      </label>
      <p className="fine-print">{template.description}{saved.some(item => item.name === template.name) ? ' You have saved it.' : ''}</p>
    </div>)}
  </section>;
}

export function BuiltInProfiles({ saved }: { saved: ProfileSummary[] }) {
  return <BuiltInProfilesPanel saved={saved} hidden={useHiddenBuiltIns()} />;
}
