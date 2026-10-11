import { useRef, useState } from 'react';
import type { DragEvent } from 'react';
import { Check, Copy, FileArchive, FileDown, FileUp } from 'lucide-react';
import { formatBytes } from './diagnostics';
import { ImportError, isMhzFileName, readImportFile, readMhzFile } from './profile-library';
import type { ProfileSession, SessionState } from './profile-session';
import { errorMessage } from './ui-model';
import { withHint } from './shortcuts';
import { useT } from './i18n-react';

// Reads the file in the webview and hands its text to the session, which
// validates everything before creating a profile. Nothing is sent anywhere.
export async function importFile(session: ProfileSession, file: Pick<File, 'size' | 'text'>) {
  let text: string;
  try { text = await readImportFile(file); }
  catch (reason) { session.setImportError(reason instanceof ImportError ? reason.message : `Could not read the file: ${errorMessage(reason)}`); return; }
  await session.importProfile(text);
}

// An official MasterHUB .mhz (or multi-profile .zip): the bytes go to the backend, which reads them in memory only.
export async function importMhzFile(session: ProfileSession, file: Pick<File, 'size' | 'arrayBuffer'>) {
  let bytes: Uint8Array;
  try { bytes = await readMhzFile(file); }
  catch (reason) { session.setImportError(reason instanceof ImportError ? reason.message : `Could not read the file: ${errorMessage(reason)}`); return; }
  session.importMhz(bytes);
}

// A dropped file is routed by its name: .mhz and .zip are official files, anything else our JSON.
export function importDropped(session: ProfileSession, file: Pick<File, 'name' | 'size' | 'text' | 'arrayBuffer'>) {
  return isMhzFileName(file.name) ? importMhzFile(session, file) : importFile(session, file);
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
        if (file && !disabled) void importDropped(session, file);
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
  const t = useT();
  const input = useRef<HTMLInputElement>(null);
  const mhzInput = useRef<HTMLInputElement>(null);
  const [copy, setCopy] = useState<string | null>(null);
  const { exportResult, importError } = state;
  return <div className="profile-transfer">
    <div className="profile-actions">
      <button className="button" disabled={disabled} title={withHint(t('transfer.duplicateTip'), 'D', mac)} onClick={() => session.requestDuplicate()}><Copy size={17} />{t('transfer.duplicate')}</button>
      <button className="button" disabled={disabled} title={t('transfer.exportTip')} onClick={() => { setCopy(null); void session.exportProfile(); }}><FileDown size={17} />{t('transfer.export')}</button>
      <button className="button" disabled={disabled} title={t('transfer.importTip')} onClick={() => input.current?.click()}><FileUp size={17} />{t('transfer.import')}</button>
      <input ref={input} type="file" accept=".json,application/json" className="sr-only" tabIndex={-1} aria-label={t('transfer.importLabel')} disabled={disabled}
        onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importFile(session, file); }} />
      <button className="button" disabled={disabled} title={t('transfer.mhzTip')} onClick={() => mhzInput.current?.click()}><FileArchive size={17} />{t('transfer.mhz')}</button>
      <input ref={mhzInput} type="file" accept=".mhz,.zip" className="sr-only" tabIndex={-1} aria-label={t('transfer.mhzLabel')} disabled={disabled}
        onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importMhzFile(session, file); }} />
    </div>
    {importError && <p className="field-error import-error" role="alert">{importError}</p>}
    {exportResult && <div className="diagnostics-result" role="status">
      <strong>{t('transfer.exported')}</strong>
      <code className="storage-path">{exportResult.path}</code>
      <p className="fine-print">{formatBytes(exportResult.bytes)} · {t('transfer.exportNote')}</p>
      <div className="diagnostics-actions"><button type="button" className="button" onClick={() => { void copyText(exportResult.path, typeof navigator === 'undefined' ? undefined : navigator.clipboard).then(setCopy); }}>{copy === 'Path copied.' ? <Check size={16} /> : <Copy size={16} />}{t('transfer.copyPath')}</button>{copy && <span className="fine-print">{copy === copyFallback ? t('transfer.copyFailed') : t('transfer.copied')}</span>}</div>
    </div>}
  </div>;
}
