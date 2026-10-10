import { useRef, useState } from 'react';
import { LoaderCircle, Trash2 } from 'lucide-react';
import type { DialDisplayContent, Profile } from './contracts';
import { prepareArtwork } from './artwork-model';
import type { PrepareResult } from './artwork-model';
import { readSourceFile } from './artwork-model';
import { DialFace } from './DisplayContent';
import { dialDisplayOptions, maxDialImageBytes, setDialDisplayContent, setDialImage } from './dial-display-model';
import type { Invoke, ProfileSession } from './profile-session';
import { SelectField } from './controls';
import { errorMessage } from './ui-model';

export const dialDisplayHeadingId = 'dial-display-heading';

// Qualities tried in order until the prepared picture fits the profile's dial
// image budget; the profile would refuse anything bigger.
const dialImageQualities = [85, 70, 55, 40, 25];

// The "Display" tab of the dial editor: what the M131E screen shows while this
// profile runs and OpenStreamHub drives the display. The choice is a profile
// setting (dial_display), so it travels with the profile and applies at the
// next run start; a running profile keeps the face it started with.
export function DialDisplayPanel({ profile, session, disabled, invoke }: { profile: Profile; session: ProfileSession; disabled: boolean; invoke?: Invoke }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);
  const content = profile.dial_display?.content ?? 'function';
  const image = profile.dial_display?.image;

  const run = (change: (latest: Profile) => Profile, key?: string) => {
    try { session.update(change, key); } catch (reason) { session.setError(errorMessage(reason)); }
  };

  // Prepares the chosen file as the exact 240×240 JPEG the dial receives,
  // lowering the quality until it fits the profile budget, then stores it in
  // the profile. Never applies across a draft change (undo, switch or save).
  const choose = async (file: File) => {
    if (!invoke) return;
    const id = ++requestId.current;
    const before = session.getState();
    setBusy(true); setError(null);
    try {
      const source = await readSourceFile(file);
      if (!source.ok) throw new Error(source.error);
      let prepared: PrepareResult | null = null;
      for (const quality of dialImageQualities) {
        prepared = await prepareArtwork(invoke, { image_base64: source.base64, target: 'dial', quality, fit: 'cover', quarter_turns: 0, background: [0, 0, 0] });
        if (prepared.bytes <= maxDialImageBytes) break;
      }
      if (!prepared || prepared.bytes > maxDialImageBytes) {
        throw new Error('This picture does not fit the dial even at the lowest quality. Choose a simpler one.');
      }
      if (id !== requestId.current) return;
      const current = session.getState();
      if (current.draftGeneration !== before.draftGeneration || current.draft.profile !== before.draft.profile) {
        setError('The profile changed while preparing the image. Choose the file again.');
        return;
      }
      session.edit(setDialImage(current.draft.profile, prepared.jpeg_base64));
    } catch (reason) {
      if (id === requestId.current) setError(errorMessage(reason));
    } finally {
      if (id === requestId.current) setBusy(false);
    }
  };

  return <section className="dial-display" aria-labelledby={dialDisplayHeadingId}>
    <h3 id={dialDisplayHeadingId}>Dial screen <span className="small-label">{dialDisplayOptions.find(option => option.value === content)?.label ?? 'Default'}</span></h3>
    <p className="fine-print">What the encoder dial shows while this profile runs and OpenStreamHub drives the display (Module control: App). The choice belongs to this profile and applies at its next start; a running profile keeps the face it started with. The screen is redrawn at most about once a second.</p>
    <label htmlFor="dial-display-mode">Dial screen shows</label>
    <SelectField id="dial-display-mode" value={content} disabled={disabled || busy} onChange={event => run(latest => setDialDisplayContent(latest, event.target.value as DialDisplayContent), 'dial-display')}>
      {dialDisplayOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </SelectField>
    {content === 'cpu' && <p className="fine-print">CPU usage is measured on this Mac; elsewhere the dial shows “--”. The preview below shows an illustrative value.</p>}
    {content === 'image' && <>
      {image
        ? <div className="dial-display-image" aria-label="Custom dial image"><img src={`data:image/jpeg;base64,${image}`} alt="" draggable={false} /><button type="button" className="button compact" disabled={disabled || busy} onClick={() => run(latest => setDialDisplayContent(latest, 'function'), 'dial-display')}><Trash2 size={16} />Remove image</button></div>
        : <p className="fine-print">Choose a picture to show; it is cropped to fill the round screen.</p>}
      <label className="spaced-label" htmlFor="dial-display-image-file">Dial picture</label>
      <input id="dial-display-image-file" type="file" accept="image/png,image/jpeg,.png,.jpg,.jpeg" disabled={disabled || busy || !invoke} onChange={event => { const file = event.target.files?.[0]; if (file) void choose(file); event.target.value = ''; }} />
      {busy && <p className="fine-print" role="status"><LoaderCircle size={15} className="spinning" /> Preparing the image…</p>}
      {!invoke && <p className="fine-print">Preparing the dial picture needs the desktop app.</p>}
    </>}
    {error && <p className="field-error" role="alert">{error}</p>}
    <div className="dial-face-preview" role="img" aria-label="Preview of the dial screen face">
      <DialFace profile={profile} />
    </div>
    <p className="fine-print">{content === 'clock' || content === 'analog_clock' ? 'The clock faces follow the time on this computer.' : content === 'cpu' ? 'A run draws the real reading.' : 'This is the face a run draws on the dial.'}</p>
  </section>;
}
