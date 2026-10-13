import { useRef, useState } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import type { Invoke } from './profile-session';
import { ModalDialog } from './controls';

export interface UpdateInfo { current: string; latest: string; available: boolean }

export const updateResultText = (info: UpdateInfo) => info.available
  ? `OpenStreamHub ${info.latest} is available. You have ${info.current}.`
  : `You are up to date (${info.current}).`;

// Settings panel for finding out about a newer release. Nothing is contacted until the button is pressed, or on
// launch when the daily check is on. The app never downloads anything: it opens the release page.
export function UpdatesPanel({ desktop, invoke, version, daily, onDaily }: {
  desktop: boolean; invoke?: Invoke; version: string | null; daily: boolean; onDaily(on: boolean): void;
}) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const call = useRef(invoke);
  call.current = invoke;
  const available = desktop && !!invoke;
  const check = async () => {
    if (!call.current) return;
    setBusy(true);
    const minimum = new Promise<void>(resolve => setTimeout(resolve, 500));
    try {
      setInfo(await call.current<UpdateInfo>('check_for_updates'));
      setError(null);
    } catch (reason) {
      setInfo(null);
      setError(String(reason));
    } finally {
      await minimum;
      setBusy(false);
    }
  };
  const openPage = () => { void call.current?.('open_release_page').catch(reason => setError(String(reason))); };
  return <section className="panel updates-panel">
    <h2>Updates</h2>
    <p>{version ? `You have version ${version}.` : 'OpenStreamHub does not update itself.'} {available ? 'Checking asks GitHub for the latest release and sends nothing else.' : ''}</p>
    {!available && <p className="fine-print">Available in the desktop app.</p>}
    {available && <div className="settings-actions">
      <button className="button" disabled={busy} aria-busy={busy} onClick={() => void check()}>{busy ? <LoaderCircle size={14} className="spinning" /> : <RefreshCw size={14} />}{busy ? 'Checking…' : 'Check for updates'}</button>
      {info?.available && <button className="button primary" onClick={openPage}>Open the release page</button>}
    </div>}
    {info && <p className="fine-print" role="status">{updateResultText(info)}</p>}
    {error && <p className="fine-print permission-error" role="alert">{error}</p>}
    {available && <><label className="setting-toggle"><input type="checkbox" checked={daily} onChange={event => onDaily(event.target.checked)} />Check once a day when the app opens</label>
      <p className="fine-print">Off by default. When a newer release exists you get one short message. Installing is up to you: download it and replace the app.</p></>}
  </section>;
}

// Shown at most once per release when the daily check finds one.
export function UpdatePrompt({ info, onOpen, onClose }: { info: UpdateInfo; onOpen(): void; onClose(): void }) {
  return <ModalDialog open labelledBy="update-title" describedBy="update-message" onCancel={onClose}>
    <h2 id="update-title">OpenStreamHub {info.latest} is available</h2>
    <p id="update-message">You have {info.current}. Download the new version from the release page and replace the app in Applications.</p>
    <div className="dialog-actions"><button className="button" autoFocus onClick={onClose}>Later</button><button className="button primary" onClick={() => { onOpen(); onClose(); }}>Open the release page</button></div>
  </ModalDialog>;
}
