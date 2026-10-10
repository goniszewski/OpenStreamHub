import { TriangleAlert } from 'lucide-react';
import type { Profile } from './contracts';
import { ModalDialog } from './controls';
import { capabilityLines, profileCapabilities } from './profile-capabilities';
import type { ConfirmCopy } from './ServicePanel';

// The one-time Live offer, shown once to a fresh install (a settings store
// with no file, so the mode reads Live with the confirmation still owed).
// Until it is answered nothing executes: the backend holds the automatic
// sessions dry, and a Run confirms the mode the same way this dialog does.
// Confirming saves Live; declining saves Test — both once, so neither the
// dialog nor the hold comes back. The header switch is always there.
export function liveOfferCopy(profile: Profile): ConfirmCopy {
  const capabilities = profileCapabilities(profile);
  return {
    title: 'Run actions on this Mac?',
    confirm: 'Run actions',
    message: `OpenStreamHub runs the actions of the open profile, "${profile.name}", whenever you use the modules. Everything listed here will really run on this Mac, without asking again. You can change this at any time with the Live/Test badge in the header.`,
    risk: capabilities.risk,
    lines: capabilityLines(capabilities),
    snippets: [],
    sequences: [],
    warning: capabilities.risk === 'high' ? `High risk. ${capabilities.riskReasons.join(' ')} With one press of a control, a profile like this can type and run commands, or send what is on screen or in the clipboard to a web page. Only continue if you wrote this profile or reviewed it and trust where it came from.` : null,
    acknowledge: capabilities.risk === 'high' ? 'I reviewed this profile and trust its source' : null,
  };
}

// Hook-free and controlled, like the other dialogs: the caller owns the open
// state, the acknowledgement and the answers. The disclosure shows the open
// profile — the one a new user is about to run; each Run still asks before
// it starts.
export function LiveOfferDialog({ open, profile, acknowledged = false, setAcknowledged, onConfirm, onDecline }: {
  open: boolean; profile: Profile; acknowledged?: boolean; setAcknowledged?(acknowledged: boolean): void;
  onConfirm(): void; onDecline(): void;
}) {
  const copy = liveOfferCopy(profile);
  return <ModalDialog open={open} role="alertdialog" labelledBy="live-offer-title" describedBy={copy.warning ? 'live-offer-warning live-offer-message' : 'live-offer-message'} onCancel={onDecline} className="execute-dialog">
    <h2 id="live-offer-title">{copy.title}</h2>
    {copy.warning && <p id="live-offer-warning" className="execute-warning"><TriangleAlert size={18} aria-hidden="true" /><span>{copy.warning}</span></p>}
    <p id="live-offer-message">{copy.message}</p>
    <div className="execute-details" tabIndex={0} role="region" aria-label="What this profile will do">
      <ul className="execute-capabilities">{copy.lines.map(line => <li key={line}>{line}</li>)}</ul>
    </div>
    {copy.acknowledge && setAcknowledged && <label className="service-option execute-acknowledge"><input type="checkbox" checked={acknowledged} onChange={event => setAcknowledged(event.target.checked)} />{copy.acknowledge}</label>}
    <div className="dialog-actions">
      <button type="button" className="button" autoFocus onClick={onDecline}>Stay in Test mode</button>
      <button type="button" className="button danger" disabled={copy.acknowledge !== null && !acknowledged} onClick={onConfirm}>{copy.confirm}</button>
    </div>
  </ModalDialog>;
}
