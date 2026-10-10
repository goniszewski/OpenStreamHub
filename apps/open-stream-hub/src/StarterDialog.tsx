import type { Profile, ServiceModule } from './contracts';
import { ModalDialog } from './controls';
import { starterModules, starters, startersForModule } from './starter-model';
import { modules } from './ui-model';

export type StarterEntry = 'first-run' | 'manager' | 'module';

const heading: Record<StarterEntry, string> = {
  'first-run': 'Set up your first profile',
  manager: 'New from a starter',
  module: 'Module connected',
};

const intro: Record<StarterEntry, string> = {
  'first-run': 'Pick a starter to begin with a saved profile of working controls, or start with an empty one. Everything stays editable.',
  manager: 'A starter becomes a new saved profile with working controls. The open profile is switched to it.',
  module: 'A module you have not seen before was connected. Start from a starter that uses it, or keep your current profile.',
};

const closeLabel: Record<StarterEntry, string> = { 'first-run': 'Start with an empty profile', manager: 'Cancel', module: 'Not now' };

// The starter chooser. One dialog serves all three entries: the first run,
// the profile manager, and a module that was just connected (then only
// starters that use that module are offered). Hook-free and controlled, so
// tests render it as a function and invoke the handlers.
export function StarterDialog({ open, entry, module, onPick, onClose, onGuide }: {
  open: boolean; entry: StarterEntry; module: ServiceModule | null;
  onPick(profile: Profile): void; onClose(): void; onGuide(): void;
}) {
  const offered = module ? startersForModule(module) : starters;
  const moduleName = modules.find(item => item.id === module)?.shortName;
  return <ModalDialog open={open} labelledBy="starter-title" describedBy="starter-body" onCancel={onClose} className="starter-dialog">
    <h2 id="starter-title">{entry === 'module' ? `${moduleName} connected` : heading[entry]}</h2>
    <p id="starter-body">{intro[entry]}</p>
    <div className="starter-cards">
      {offered.map(item => <button key={item.id} type="button" className="starter-card" data-starter={item.id} onClick={() => onPick(item.profile)}>
        <strong>{item.name}</strong>
        <span>{item.description}</span>
        <small>Sets up: {starterModules(item.profile).map(id => modules.find(candidate => candidate.id === id)?.shortName ?? id).join(', ')}</small>
      </button>)}
    </div>
    <p className="fine-print">Starters hold nothing that types into apps, opens programs or web pages. A run starts as a dry run that only logs what would happen; executing actions always asks for confirmation first.</p>
    <div className="dialog-actions">
      {entry === 'first-run' && <button type="button" className="button starter-guide" onClick={onGuide}>Getting started guide</button>}
      <span className="spacer" />
      <button type="button" className="button" autoFocus onClick={onClose}>{closeLabel[entry]}</button>
    </div>
  </ModalDialog>;
}
