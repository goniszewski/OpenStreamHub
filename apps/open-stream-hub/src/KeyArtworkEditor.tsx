import { useEffect, useRef, useState } from 'react';
import type { KeyArtwork } from './contracts';
import { readSourceFile } from './artwork-model';
import { keyArtworkFrame, setKeyArtwork } from './key-artwork';
import type { Invoke, ProfileSession } from './profile-session';
import { errorMessage } from './ui-model';

export function KeyArtworkPreview({ artwork }: { artwork: KeyArtwork }) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(0);
    if (artwork.frames.length < 2) return;
    const start = performance.now();
    const timer = setInterval(() => setElapsed(performance.now() - start), 100);
    return () => clearInterval(timer);
  }, [artwork]);
  const frame = artwork.frames[keyArtworkFrame(artwork, elapsed)];
  return frame ? <img className="key-artwork-image" src={`data:image/jpeg;base64,${frame.jpeg_base64}`} alt="Key artwork" width={72} height={72} /> : null;
}

export function KeyArtworkEditor({ session, path, control, artwork, disabled, invoke }: {
  session: ProfileSession; path: string[]; control: number; artwork?: KeyArtwork; disabled: boolean; invoke?: Invoke;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);
  useEffect(() => () => { requestId.current++; }, []);
  async function choose(file: File) {
    const id = ++requestId.current;
    const before = session.getState();
    setBusy(true); setError(null);
    try {
      const source = await readSourceFile(file);
      if (!source.ok) throw new Error(source.error);
      const prepared = await invoke!<KeyArtwork>('prepare_key_artwork', { request: {
        image_base64: source.base64, target: 'keypad', quality: 85, fit: 'contain', quarter_turns: 0, background: [0, 0, 0],
      } });
      if (id !== requestId.current) return;
      // Never apply completion to a different draft, nor undo intervening edits.
      const current = session.getState();
      if (current.draftGeneration !== before.draftGeneration || current.draft.profile !== before.draft.profile) {
        setError('The profile changed while preparing artwork. Choose the file again.'); return;
      }
      session.edit(setKeyArtwork(current.draft.profile, path, control, prepared));
    } catch (reason) { if (id === requestId.current) setError(errorMessage(reason)); }
    finally { if (id === requestId.current) setBusy(false); }
  }
  return <section className="key-artwork-editor" aria-label="Key artwork">
    <label htmlFor="key-artwork-file">Key image or GIF</label>
    <input id="key-artwork-file" type="file" accept="image/png,image/jpeg,image/gif,.png,.jpg,.jpeg,.gif" disabled={disabled || busy || !invoke} onChange={event => {
      const file = event.target.files?.[0]; event.target.value = ''; if (file) void choose(file);
    }} />
    {busy && <p role="status">Preparing key artwork…</p>}
    {error && <p className="field-error" role="alert">{error}</p>}
    {artwork && <><KeyArtworkPreview artwork={artwork} /><p className="fine-print">{artwork.frames.length > 1 ? `${artwork.frames.length} frames · ${artwork.loop_count === 0 ? 'loops continuously' : `${artwork.loop_count} play(s)`}` : 'Static image'}</p><button className="button compact" disabled={disabled || busy} onClick={() => session.edit(setKeyArtwork(session.getState().draft.profile, path, control, null))}>Remove artwork</button></>}
    <p className="fine-print">PNG, JPEG or GIF · up to 8 MiB. GIFs: up to 512×512 and 120 frames, at most 10 FPS. Artwork is saved with the profile and sent while OpenStreamHub drives the display.</p>
    {!invoke && <p className="fine-print">Preparing key artwork needs the desktop app.</p>}
  </section>;
}
