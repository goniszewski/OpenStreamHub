import { useRef, useState } from 'react';
import type { DragEvent } from 'react';
import { Check, Copy, FileDown, FileUp } from 'lucide-react';
import { formatBytes } from './diagnostics';
import { ImportError, readImportFile } from './profile-library';
import type { ProfileSession, SessionState } from './profile-session';
import { errorMessage } from './ui-model';
import { withHint } from './shortcuts';

// Reads the file in the webview and hands its text to the session, which
// validates everything before creating a profile. Nothing is sent anywhere.
export async function importFile(session: ProfileSession, file: Pick<File, 'size' | 'text'>) {
  let text: string;
  try { text = await readImportFile(file); }
  catch (reason) { session.setImportError(reason instanceof ImportError ? reason.message : `Could not read the file: ${errorMessage(reason)}`); return; }
  await session.importProfile(text);
}

export function useImportDrop(session: ProfileSession, disabled: boolean) {
  const [dragging, setDragging] = useState(false);
  const hasFile = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes('Files');
  return {
    dragging,
    props: {
      onDragOver: (event: DragEvent) => { if (hasFile(event)) { event.preventDefault(); if (!disabled) setDragging(true); } },
      onDragLeave: (event: DragEvent) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false); },
      onDrop: (event: DragEvent) => {
        if (!hasFile(event)) return;
        event.preventDefault(); setDragging(false);
        const file = event.dataTransfer.files[0];
        if (file && !disabled) void importFile(session, file);
      },
    },
  };
}

export const copyFallback = 'Could not copy automatically. Select the path above and copy it.';

export async function copyText(text: string, clipboard: Pick<Clipboard, 'writeText'> | undefined): Promise<string> {
  try { if (!clipboard) throw new Error('no clipboard'); await clipboard.writeText(text); return 'Path copied.'; }
  catch { return copyFallback; }
}

export function ProfileTransfer({ session, state, disabled, mac }: { session: ProfileSession; state: SessionState; disabled: boolean; mac: boolean }) {
  const input = useRef<HTMLInputElement>(null);
  const [copy, setCopy] = useState<string | null>(null);
  const { exportResult, importError } = state;
  return <div className="profile-transfer">
    <div className="profile-actions">
      <button className="button" disabled={disabled} title={withHint('Create a saved copy of this profile', 'D', mac)} onClick={() => session.requestDuplicate()}><Copy size={17} />Duplicate</button>
      <button className="button" disabled={disabled} title="Write this profile to a JSON file in the app's exports folder" onClick={() => { setCopy(null); void session.exportProfile(); }}><FileDown size={17} />Export</button>
      <button className="button" disabled={disabled} title="Import a profile from a JSON file. You can also drop a file onto this page." onClick={() => input.current?.click()}><FileUp size={17} />Import</button>
      <input ref={input} type="file" accept=".json,application/json" className="sr-only" tabIndex={-1} aria-label="Import profile JSON file" disabled={disabled}
        onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importFile(session, file); }} />
    </div>
    {importError && <p className="field-error import-error" role="alert">{importError}</p>}
    {exportResult && <div className="diagnostics-result" role="status">
      <strong>Profile exported</strong>
      <code className="storage-path">{exportResult.path}</code>
      <p className="fine-print">{formatBytes(exportResult.bytes)} · stays on this Mac · the newest 20 exports are kept.</p>
      <div className="diagnostics-actions"><button type="button" className="button" onClick={() => { void copyText(exportResult.path, typeof navigator === 'undefined' ? undefined : navigator.clipboard).then(setCopy); }}>{copy === 'Path copied.' ? <Check size={16} /> : <Copy size={16} />}Copy path</button>{copy && <span className="fine-print">{copy}</span>}</div>
    </div>}
  </div>;
}
