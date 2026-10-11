import { useEffect, useRef, useState } from 'react';
import { LoaderCircle, RotateCcw, RotateCw, Trash2 } from 'lucide-react';
import type { DialDisplayContent, Profile } from './contracts';
import { fitInfo, formatHexColour, parseHexColour, prepareArtwork, readSourceFile, rotationLabel } from './artwork-model';
import type { ArtworkFit, PrepareResult } from './artwork-model';
import { ColourSwatchPicker } from './ColourPicker';
import { sourceToKeep } from './dial-source';
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
  // The picture chosen in this session and how it is fitted. The profile keeps only the prepared JPEG, so
  // fit, rotation and background can be changed only while the source is still at hand.
  const stored = profile.dial_display?.source;
  const [source, setSource] = useState<string | null>(stored?.image ?? null);
  const [fit, setFit] = useState<ArtworkFit>(stored?.fit ?? 'cover');
  const [turns, setTurns] = useState(stored?.quarter_turns ?? 0);
  const [background, setBackground] = useState(stored ? formatHexColour(stored.background) : '#000000');
  // The kept source is the truth after a reopen, an undo or a profile switch.
  useEffect(() => {
    if (!stored) return;
    setSource(stored.image); setFit(stored.fit); setTurns(stored.quarter_turns); setBackground(formatHexColour(stored.background));
  }, [stored]);
  const content = profile.dial_display?.content ?? 'function';
  const image = profile.dial_display?.image;

  const run = (change: (latest: Profile) => Profile, key?: string) => {
    try { session.update(change, key); } catch (reason) { session.setError(errorMessage(reason)); }
  };

  // Prepares the source as the exact 240×240 JPEG the dial receives, lowering the quality until it fits the
  // profile budget, then stores it in the profile. Never applies across a draft change (undo, switch or save).
  const prepare = async (base64: string, look: { fit: ArtworkFit; turns: number; background: string }, keep: string | null) => {
    if (!invoke) return;
    const rgb = parseHexColour(look.background);
    if (!rgb) return;
    const id = ++requestId.current;
    const before = session.getState();
    setBusy(true); setError(null);
    try {
      let prepared: PrepareResult | null = null;
      for (const quality of dialImageQualities) {
        prepared = await prepareArtwork(invoke, { image_base64: base64, target: 'dial', quality, fit: look.fit, quarter_turns: look.turns, background: rgb });
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
      session.edit(setDialImage(current.draft.profile, prepared.jpeg_base64, keep ? { image: keep, fit: look.fit, quarter_turns: look.turns, background: rgb } : undefined));
    } catch (reason) {
      if (id === requestId.current) setError(errorMessage(reason));
    } finally {
      if (id === requestId.current) setBusy(false);
    }
  };

  const choose = async (file: File) => {
    setBusy(true); setError(null);
    const read = await readSourceFile(file);
    if (!read.ok) { setBusy(false); setError(read.error); return; }
    setSource(read.base64);
    const kept = await sourceToKeep(read.base64, read.mime, background);
    await prepare(read.base64, { fit, turns, background }, kept?.image ?? null);
  };

  // A change of look re-prepares the chosen picture.
  const adjust = (next: Partial<{ fit: ArtworkFit; turns: number; background: string }>) => {
    const look = { fit, turns, background, ...next };
    setFit(look.fit); setTurns(look.turns); setBackground(look.background);
    if (source) void prepare(source, look, stored ? source : null);
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
        ? <div className="dial-display-image" aria-label="Custom dial image"><img src={`data:image/jpeg;base64,${image}`} alt="" draggable={false} /><button type="button" className="button compact" disabled={disabled || busy} onClick={() => { setSource(null); run(latest => setDialDisplayContent(latest, 'function'), 'dial-display'); }}><Trash2 size={16} />Remove image</button></div>
        : <p className="fine-print">Choose a picture to show; it is cropped to fill the round screen.</p>}
      <label className="spaced-label" htmlFor="dial-display-image-file">Dial picture</label>
      <input id="dial-display-image-file" type="file" accept="image/png,image/jpeg,.png,.jpg,.jpeg" disabled={disabled || busy || !invoke} onChange={event => { const file = event.target.files?.[0]; if (file) void choose(file); event.target.value = ''; }} />
      {source ? <div className="dial-image-look">
        <label htmlFor="dial-image-fit">Fit</label>
        <SelectField id="dial-image-fit" value={fit} disabled={disabled || busy} onChange={event => adjust({ fit: event.target.value as ArtworkFit })}>
          {(Object.keys(fitInfo) as ArtworkFit[]).map(key => <option key={key} value={key}>{fitInfo[key].label}</option>)}
        </SelectField>
        <p className="fine-print">{fitInfo[fit].help}</p>
        <label>Rotation</label>
        <div className="dial-image-rotate">
          <button type="button" className="button compact" aria-label="Rotate left" disabled={disabled || busy} onClick={() => adjust({ turns: (turns + 3) % 4 })}><RotateCcw size={16} /></button>
          <span aria-live="polite">{rotationLabel(turns)}</span>
          <button type="button" className="button compact" aria-label="Rotate right" disabled={disabled || busy} onClick={() => adjust({ turns: (turns + 1) % 4 })}><RotateCw size={16} /></button>
        </div>
        <label htmlFor="dial-image-background">Background colour</label>
        <div className="artwork-colour">
          <input id="dial-image-background" type="text" value={background} disabled={disabled || busy} spellCheck={false} autoComplete="off" maxLength={7}
            aria-invalid={parseHexColour(background) === null} onChange={event => adjust({ background: event.target.value })} />
          <ColourSwatchPicker label="Pick dial background colour" value={formatHexColour(parseHexColour(background) ?? [0, 0, 0])} disabled={disabled || busy} onChange={hex => adjust({ background: hex })} />
        </div>
        <p className="fine-print">Fills the margins when the picture does not cover the whole screen.</p>
      </div> : image && <p className="fine-print">To change the fit, rotation or background, choose the picture again.</p>}
      {source && !stored && image && <p className="fine-print">This picture is too large to keep in the profile, so these controls work only until the app closes.</p>}
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
