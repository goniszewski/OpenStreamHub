import { useState } from 'react';
import { ShieldAlert, TriangleAlert } from 'lucide-react';
import type { ImportCounts, ImportOutcome, ImportedProfile, MhzImportResult, Profile } from './contracts';
import { ModalDialog } from './controls';
import { capabilityLines, profileCapabilities, snippetPreview, stepLabel } from './profile-capabilities';

// The official .mhz import report and the explicit step that lets an imported profile execute actions.

export const outcomeLabels: Record<ImportOutcome, string> = {
  imported: 'Imported',
  imported_lossy: 'Imported with changes',
  unbound_unsupported: 'Left unbound',
  unbound_dangerous: 'Left unbound (dangerous)',
};

export function countsLine(counts: ImportCounts): string {
  return (Object.keys(outcomeLabels) as ImportOutcome[]).map(outcome => `${outcomeLabels[outcome]}: ${counts[outcome]}`).join(' · ');
}

export const dryRunOnlyNote = 'Imported profiles run only as a dry run until you allow them to execute actions in the profile manager (Manage profiles, next to the profile switcher).';

function ImportedSummary({ item }: { item: ImportedProfile }) {
  const { report, stored } = item;
  const lines = capabilityLines(profileCapabilities(stored.profile));
  return <section className="mhz-report" aria-label={`Import report for ${report.profile_name}`}>
    <h3>{report.profile_name}</h3>
    {/* Row risk follows the import rules; the allow confirmation rates the whole profile with profile-capabilities,
        which can be stricter (for example any key pressed after pasted text). */}
    <p className="fine-print">{countsLine(report.totals)}{report.high_risk > 0 ? ` · High risk: ${report.high_risk}` : ''}. The confirmation before allowing actions lists the full risk.</p>
    {report.warnings.length > 0 && <ul className="mhz-warnings">{report.warnings.map(warning => <li key={warning}>{warning}</li>)}</ul>}
    <h4>Once allowed, this profile</h4>
    <ul className="execute-capabilities">{lines.map(line => <li key={line}>{line}</li>)}</ul>
    <h4>Every control</h4>
    {report.rows.length === 0 ? <p className="fine-print">The file binds no controls.</p>
      : <div className="table-scroll"><table className="events-table mhz-rows"><thead><tr><th>Control</th><th>Official action</th><th>Result</th><th>Why</th></tr></thead>
        <tbody>{report.rows.map((row, index) => <tr key={index} className={row.outcome}>
          <td>{row.path}</td><td><code>{row.official_action}</code></td>
          <td>{outcomeLabels[row.outcome]}{row.high_risk && <strong className="execute-enter"> High risk</strong>}</td>
          <td>{row.reason}{row.detail && <small> ({row.detail})</small>}</td>
        </tr>)}</tbody></table></div>}
  </section>;
}

export function MhzImportReport({ result, onClose }: { result: MhzImportResult | null; onClose(): void }) {
  return <ModalDialog open={result !== null} labelledBy="mhz-report-title" describedBy="mhz-report-message" onCancel={onClose} className="execute-dialog mhz-dialog">
    <h2 id="mhz-report-title">Imported from MasterHUB</h2>
    <p id="mhz-report-message" className="execute-warning"><ShieldAlert size={18} aria-hidden="true" /><span>{dryRunOnlyNote} Nothing from the file has run. Programs, macros, scripts, integrations and images were not imported.</span></p>
    <div className="execute-details" tabIndex={0} role="region" aria-label="Import report">
      {result?.profiles.map(item => <ImportedSummary key={item.stored.id} item={item} />)}
    </div>
    <div className="dialog-actions"><button type="button" className="button" autoFocus onClick={onClose}>Close</button></div>
  </ModalDialog>;
}

// Copy for the confirmation that clears the dry-run-only flag. It names the profile and lists what it can do.
export function allowCopy(name: string, profile: Profile) {
  const capabilities = profileCapabilities(profile);
  return {
    title: `Allow "${name}" to execute actions?`,
    message: `"${name}" was imported from a MasterHUB .mhz file, so it has only run as a dry run. After this, it can run with actions like any profile you made. Starting a run with actions still asks for confirmation each time.`,
    risk: capabilities.risk,
    warning: capabilities.risk === 'high' ? `High risk. ${capabilities.riskReasons.join(' ')}` : null,
    lines: capabilityLines(capabilities),
    snippets: capabilities.texts.map(snippet => ({ ...snippetPreview(snippet.text), pressEnter: snippet.pressEnter })),
    sequences: capabilities.sequences.map(steps => steps.map(stepLabel)),
    acknowledge: 'I reviewed every binding of this imported profile and trust its source',
  };
}

export function AllowExecutionDialog({ target, onCancel, onConfirm }: {
  target: { name: string; profile: Profile } | null; onCancel(): void; onConfirm(): void;
}) {
  const [acknowledged, setAcknowledged] = useState(false);
  const copy = target ? allowCopy(target.name, target.profile) : null;
  const close = (confirmed: boolean) => { setAcknowledged(false); if (confirmed) onConfirm(); else onCancel(); };
  return <ModalDialog open={copy !== null} role="alertdialog" labelledBy="allow-title" describedBy="allow-message" onCancel={() => close(false)} className="execute-dialog">
    {copy && <>
      <h2 id="allow-title">{copy.title}</h2>
      {copy.warning && <p className="execute-warning"><TriangleAlert size={18} aria-hidden="true" /><span>{copy.warning}</span></p>}
      <p id="allow-message">{copy.message}</p>
      <div className="execute-details" tabIndex={0} role="region" aria-label="What this profile will do">
        <ul className="execute-capabilities">{copy.lines.map(line => <li key={line}>{line}</li>)}</ul>
        {copy.snippets.length > 0 && <>
          <h3>Text snippets</h3>
          <ol className="execute-snippets">{copy.snippets.map((snippet, index) => <li key={index}>
            <code>{snippet.text}</code>
            {snippet.more > 0 && <span className="fine-print"> … and {snippet.more} more characters not shown</span>}
            {snippet.pressEnter && <strong className="execute-enter"> Then presses Return.</strong>}
          </li>)}</ol>
        </>}
        {copy.sequences.length > 0 && <>
          <h3>Multi-step sequences</h3>
          {copy.sequences.map((steps, index) => <ol key={index} className="execute-sequence" aria-label={`Sequence ${index + 1}`}>{steps.map((step, stepIndex) => <li key={stepIndex}>{step}</li>)}</ol>)}
        </>}
      </div>
      <label className="service-option execute-acknowledge"><input type="checkbox" checked={acknowledged} onChange={event => setAcknowledged(event.target.checked)} />{copy.acknowledge}</label>
      <div className="dialog-actions"><button type="button" className="button" autoFocus onClick={() => close(false)}>Cancel</button><button type="button" className="button danger" disabled={!acknowledged} onClick={() => close(true)}>Allow actions</button></div>
    </>}
  </ModalDialog>;
}
