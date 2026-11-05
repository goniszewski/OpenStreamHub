import { useCallback, useEffect, useRef, useState } from 'react';
import { Check, Circle, Copy, LoaderCircle, RefreshCw } from 'lucide-react';
import { PATH_EXPORT, agentChecklist, agentTargets, copyText, oshState, readAgentSignals, runAgentTool, showPathHint, signalsSeen } from './agent-tools-model';
import type { AgentSignalsReply, AgentToolCommand, AgentToolStatus } from './agent-tools-model';
import { useT } from './i18n-react';
import type { Invoke } from './profile-session';

// A copy button whose label turns to "Copied" for a moment, or reports that copying failed.
function CopyButton({ text, label, copy }: { text: string; label: string; copy(text: string): Promise<boolean> }) {
  const t = useT();
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  const press = async () => {
    const copied = await copy(text);
    setState(copied ? 'copied' : 'failed');
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setState('idle'), 2000);
  };
  return <>
    <button type="button" className="button compact" aria-label={label} onClick={() => void press()}>{state === 'copied' ? <Check size={14} /> : <Copy size={14} />}{state === 'copied' ? t('agents.copied') : t('agents.copy')}</button>
    {state === 'failed' && <span className="fine-print permission-error" role="alert">{t('agents.copyFailed')}</span>}
  </>;
}

// First-run checklist: done steps get a check, the others a circle with a hint underneath and an action on the right.
// Collapses to one line when all are done.
function SetupChecklist({ status, signals, copy }: { status: AgentToolStatus; signals: AgentSignalsReply | null; copy(text: string): Promise<boolean> }) {
  const t = useT();
  const { steps, ready } = agentChecklist(status, signalsSeen(signals));
  if (ready) return <p className="fine-print agent-tools-ready" role="status"><Check size={14} /> {t('agents.check.ready')}</p>;
  const hookHint = (step: 'claude' | 'codex') => {
    if (!signals) return t('agents.check.notChecked');
    const file = agentTargets.find(target => target.agent === step)!.file;
    return t(signals[step] === 'unreadable' ? 'agents.check.hook.unreadable' : 'agents.check.hook.missing', { file });
  };
  return <div className="agent-tools-checklist">
    <h3>{t('agents.check.title')}</h3>
    <ul>
      {steps.map(({ step, done }) => {
        const name = t(step === 'codex' ? 'agents.codex.title' : 'agents.claude.title');
        const hint = done ? null
          : step === 'osh' ? t(status.translocated ? 'agents.check.osh.translocated' : 'agents.check.osh.missing')
          : step === 'answered' ? t('agents.check.answered.hint')
          : hookHint(step);
        return <li key={step} className={`agent-tools-step agent-tools-step-${step}`} data-done={done}>
          {done ? <Check size={14} aria-label={t('agents.check.done')} /> : <Circle size={14} aria-label={t('agents.check.todo')} />}
          <div className="agent-tools-step-text">
            <span>{t(`agents.check.${step}`)}</span>
            {hint && <span className="fine-print">{hint}</span>}
          </div>
          <div className="agent-tools-step-action">
            {!done && (step === 'claude' || step === 'codex') && status.osh_path && !status.translocated && <CopyButton text={agentTargets.find(target => target.agent === step)!.config(status)} label={t('agents.copyConfig', { name })} copy={copy} />}
          </div>
        </li>;
      })}
    </ul>
    <p className="fine-print agent-tools-caveat">{t('agents.check.caveat')}</p>
  </div>;
}

// Settings panel for the AI agent hooks: finds the bundled osh, links it into ~/.local/bin on request and shows the
// Claude Code and Codex hook configurations to paste. The agents' settings files are never edited by the app.
export function AgentToolsPanel({ desktop, invoke, copy = copyText }: { desktop: boolean; invoke?: Invoke; copy?(text: string): Promise<boolean> }) {
  const t = useT();
  const [status, setStatus] = useState<AgentToolStatus | null>(null);
  const [signals, setSignals] = useState<AgentSignalsReply | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const available = desktop && !!invoke;
  // The parent passes a new function on every render; keep the latest in a ref so a re-render never re-runs the check.
  const call = useRef(invoke);
  call.current = invoke;
  const run = useCallback(async (command: AgentToolCommand, done?: string) => {
    if (!call.current) return;
    setBusy(true);
    try {
      setStatus(await runAgentTool(call.current, command));
      // The checklist's own readings are refreshed with the status (on mount and on "Check again"); a failure leaves it unchecked.
      if (command === 'agent_tool_status') setSignals(await readAgentSignals(call.current).catch(() => null));
      setError(null);
      setResult(done ?? null);
    } catch (reason) {
      setError(String(reason));
      setResult(null);
    } finally {
      setBusy(false);
    }
  }, []);
  useEffect(() => { if (available) void run('agent_tool_status'); }, [available, run]);

  const state = status ? oshState(status) : null;
  return <section className="panel agent-tools-panel">
    <h2>{t('agents.title')}</h2>
    <p>{t('agents.intro')}</p>
    {!available && <p className="fine-print">{t('agents.desktopOnly')}</p>}
    {available && !status && !error && <p className="fine-print" role="status">{t('agents.loading')}</p>}
    {status && <SetupChecklist status={status} signals={signals} copy={copy} />}
    {status && state === 'translocated' && <p className="fine-print permission-error" role="status">{t('agents.osh.translocated')}</p>}
    {status && state === 'missing' && <p className="fine-print permission-error" role="status">{t('agents.osh.missing')}</p>}
    {status && status.osh_path && !status.translocated && <>
      <label>{t('agents.osh.found')}</label>
      <code className="storage-path">{status.osh_path}</code>
      <h3>{t('agents.link.title')}</h3>
      <p className="fine-print" role="status">{state === 'linked' ? t('agents.link.linked', { path: status.link_path }) : t('agents.link.unlinked')}</p>
      <div className="settings-actions">
        {state === 'linked'
          ? <button type="button" className="button" disabled={busy} aria-busy={busy} onClick={() => void run('unlink_agent_tool', t('agents.link.removed'))}>{busy ? t('agents.link.working') : t('agents.link.remove')}</button>
          : <button type="button" className="button primary" disabled={busy} aria-busy={busy} onClick={() => void run('link_agent_tool', t('agents.link.made'))}>{busy ? t('agents.link.working') : t('agents.link.make')}</button>}
        <button type="button" className="button" disabled={busy} onClick={() => void run('agent_tool_status')}>{busy ? <LoaderCircle size={14} className="spinning" /> : <RefreshCw size={14} />}{t('agents.checkAgain')}</button>
      </div>
      {result && <p className="fine-print agent-tools-result" role="status">{result}</p>}
      <p className="fine-print">{t('agents.link.note', { path: status.link_path })}</p>
      {showPathHint(status) && <div className="agent-tools-path">
        <p className="fine-print">{t('agents.path.hint')}</p>
        <div className="agent-tools-copy"><code className="storage-path">{PATH_EXPORT}</code><CopyButton text={PATH_EXPORT} label={t('agents.copyPath')} copy={copy} /></div>
      </div>}
      {agentTargets.map(target => {
        const name = t(target.agent === 'claude' ? 'agents.claude.title' : 'agents.codex.title');
        return <div key={target.agent} className={`agent-tools-target agent-tools-${target.agent}`}>
          <h3>{name}</h3>
          <p className="fine-print">{t('agents.target', { file: target.file })}</p>
          <pre className="storage-path agent-tools-config" tabIndex={0}>{target.config(status)}</pre>
          <div className="settings-actions"><CopyButton text={target.config(status)} label={t('agents.copyConfig', { name })} copy={copy} /></div>
          <p className="fine-print">{t(target.agent === 'claude' ? 'agents.claude.merge' : 'agents.codex.merge')}</p>
          {target.agent === 'codex' && <p className="fine-print">{t('agents.codex.trust')}</p>}
        </div>;
      })}
      <p className="fine-print">{t('agents.manual')}</p>
      <p className="fine-print">{t('agents.starter')}</p>
    </>}
    {error && <p className="fine-print permission-error" role="alert">{error}</p>}
  </section>;
}
