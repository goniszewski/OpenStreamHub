import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, DragEvent } from 'react';
import { Check, Copy, FileDown, ImageUp, LoaderCircle, TriangleAlert } from 'lucide-react';
import {
  artworkName, buildArtworkRequest, buildSaveRequest, clampQuality, createLatestRunner, dataUrl, debounceMs, defaultArtworkOptions, fitInfo,
  formatHexColour, maxSourceBytes, offlineNote, parseHexColour, prepareArtwork, readSourceFile, rotationLabel, rotations, saveArtwork, targetInfo,
} from './artwork-model';
import type { ArtworkFit, ArtworkOptions, ArtworkTarget, PrepareResult, SaveResult } from './artwork-model';
import { formatBytes } from './diagnostics';
import { moduleArtwork } from './module-controls';
import type { Invoke } from './profile-session';
import { copyText } from './ProfileTransfer';

export interface ArtworkSource { name: string; base64: string; mime: string; size: number }

// The encoder screen has the same two-level geometry as the main hero in
// DevicesPage: the single-display box (20/14/60/56 of the artwork), and inside it
// the round screen from `.encoder-display` in styles.css (17/16/65/63).
const encoderBox = { left: 20, top: 14, width: 60, height: 56 };

// The prepared JPEG fills the display area (cover, clipped to the round screen
// for the dial); the stage only scales it.
function ArtworkStage({ target, src, busy }: { target: ArtworkTarget; src: string | null; busy: boolean }) {
  const dial = target === 'dial';
  const key = moduleArtwork.m4315.controls[0];
  const area = dial ? encoderBox : key;
  const style: CSSProperties = { left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` };
  const display = <div className={`artwork-display${dial ? ' round' : ''}${busy ? ' busy' : ''}`} style={dial ? undefined : style}>
    {src ? <img src={src} alt="" draggable={false} /> : <span className="artwork-display-empty" aria-hidden="true" />}
  </div>;
  return <div className={`artwork-stage ${dial ? 'dial' : 'keypad'}`} style={{ '--art-aspect': dial ? 1 : moduleArtwork.m4315.aspect } as CSSProperties}
    role="img" aria-label={dial ? 'Prepared image on the encoder display' : 'Prepared image on key 1 of the 15-key display'}>
    <img className="artwork-module" src={`/modules/${dial ? 'encoder' : 'keys'}.png`} alt="" draggable={false} />
    {dial ? <div className="artwork-dial-box" style={style}>{display}</div> : display}
  </div>;
}

export function ArtworkPage({ desktop, invoke, initialSource, initialResult, initialSaved, initialOptions }: {
  desktop: boolean; invoke: Invoke; initialSource?: ArtworkSource; initialResult?: PrepareResult; initialSaved?: SaveResult; initialOptions?: Partial<ArtworkOptions>;
}) {
  const [options, setOptions] = useState<ArtworkOptions>({ ...defaultArtworkOptions, ...initialOptions });
  const [source, setSource] = useState<ArtworkSource | null>(initialSource ?? null);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [result, setResult] = useState<PrepareResult | null>(initialResult ?? null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [saved, setSaved] = useState<SaveResult | null>(initialSaved ?? null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [copy, setCopy] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const previews = useRef(createLatestRunner());
  const set = <K extends keyof ArtworkOptions>(key: K, value: ArtworkOptions[K]) => setOptions(current => ({ ...current, [key]: value }));

  const built = useMemo(() => buildArtworkRequest(source?.base64 ?? null, options), [source, options]);
  const colour = parseHexColour(options.background);
  const colourInvalid = colour === null;
  const locked = !desktop;

  // Debounced live preview. Only the newest request may update the screen.
  const request = built.ok ? built.request : null;
  useEffect(() => {
    const runner = previews.current;
    if (!desktop || !request) { runner.cancel(); setLoading(false); if (!request) { setResult(null); setPreviewError(null); } return; }
    setLoading(true);
    const timer = setTimeout(() => {
      void runner.run(() => prepareArtwork(invoke, request)).then(outcome => {
        if (!outcome) return;
        setLoading(false);
        if (outcome.ok) { setResult(outcome.value); setPreviewError(null); } else { setResult(null); setPreviewError(outcome.error); }
      });
    }, debounceMs);
    return () => { clearTimeout(timer); runner.cancel(); };
  }, [desktop, invoke, request]);

  const load = async (file: File | undefined) => {
    if (!file) return;
    const outcome = await readSourceFile(file);
    if (!outcome.ok) { setSourceError(outcome.error); return; }
    setSourceError(null); setSaved(null); setSaveError(null); setCopy(null);
    setSource({ name: file.name, base64: outcome.base64, mime: outcome.mime, size: outcome.size });
  };

  const hasFile = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes('Files');
  const drop = {
    onDragOver: (event: DragEvent) => { if (hasFile(event)) { event.preventDefault(); setDragging(true); } },
    onDragLeave: (event: DragEvent) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false); },
    onDrop: (event: DragEvent) => { if (!hasFile(event)) return; event.preventDefault(); setDragging(false); void load(event.dataTransfer.files[0]); },
  };

  const exportJpeg = async () => {
    if (!desktop || !request || !source || saving) return;
    setSaving(true); setSaveError(null); setCopy(null);
    try { setSaved(await saveArtwork(invoke, buildSaveRequest(request, artworkName(source.name)))); }
    catch (reason) { setSaved(null); setSaveError(reason instanceof Error ? reason.message : String(reason)); }
    setSaving(false);
  };

  const previewSrc = result ? dataUrl('image/jpeg', result.jpeg_base64) : null;
  const { size } = targetInfo[options.target];

  return <section className="page-content artwork-page" aria-labelledby="artwork-title">
    <div className="page-title"><div><h1 id="artwork-title">Display artwork</h1><p>Prepare a PNG or JPEG for a module display and see the exact result.</p></div><span className="mode-pill">Offline only</span></div>
    <div className="artwork-offline" role="note"><TriangleAlert size={19} /><strong>{offlineNote}</strong></div>
    {!desktop && <p className="fine-print" role="status">Preparing and exporting artwork needs the desktop app. You can still look at a source image here.</p>}
    <div className="artwork-layout">
      <div className="artwork-controls">
        <section className="panel" aria-labelledby="artwork-source-title">
          <h2 id="artwork-source-title">Source image</h2>
          <div className={`artwork-drop${dragging ? ' active' : ''}`} {...drop}>
            {source ? <img className="artwork-thumb" src={dataUrl(source.mime, source.base64)} alt={`Source image ${source.name}`} /> : <ImageUp size={26} aria-hidden="true" />}
            <div className="artwork-drop-text">
              <strong>{source ? source.name : 'Drop a PNG or JPEG here'}</strong>
              <small>{source ? formatBytes(source.size) : `Up to ${formatBytes(maxSourceBytes)}. Stays on this Mac.`}</small>
            </div>
            <button type="button" className="button" onClick={() => input.current?.click()}><ImageUp size={17} />{source ? 'Replace' : 'Choose image'}</button>
            <input ref={input} type="file" accept="image/png,image/jpeg,.png,.jpg,.jpeg" className="sr-only" tabIndex={-1} aria-label="Source image file"
              onChange={event => { const file = event.target.files?.[0]; event.target.value = ''; void load(file); }} />
          </div>
          {sourceError && <p className="field-error" role="alert">{sourceError}</p>}
        </section>
        <section className="panel" aria-labelledby="artwork-settings-title">
          <h2 id="artwork-settings-title">Settings</h2>
          <fieldset className="artwork-field" disabled={locked}>
            <legend>Target</legend>
            <div className="artwork-seg" role="group" aria-label="Target">
              {(Object.keys(targetInfo) as ArtworkTarget[]).map(value => <button key={value} type="button" aria-pressed={options.target === value} className={options.target === value ? 'active' : ''} onClick={() => set('target', value)}>{targetInfo[value].label} {targetInfo[value].size}×{targetInfo[value].size}</button>)}
            </div>
          </fieldset>
          <fieldset className="artwork-field" disabled={locked}>
            <legend>Fit</legend>
            <div className="artwork-seg" role="group" aria-label="Fit">
              {(Object.keys(fitInfo) as ArtworkFit[]).map(value => <button key={value} type="button" aria-pressed={options.fit === value} className={options.fit === value ? 'active' : ''} onClick={() => set('fit', value)}>{fitInfo[value].label}</button>)}
            </div>
            <p className="fine-print">{fitInfo[options.fit].help}</p>
          </fieldset>
          <fieldset className="artwork-field" disabled={locked}>
            <legend>Rotation (clockwise)</legend>
            <div className="artwork-seg" role="group" aria-label="Rotation">
              {rotations.map(value => <button key={value} type="button" aria-pressed={options.quarterTurns === value} className={options.quarterTurns === value ? 'active' : ''} onClick={() => set('quarterTurns', value)}>{rotationLabel(value)}</button>)}
            </div>
          </fieldset>
          <div className="artwork-field">
            <label htmlFor="artwork-background">Background colour</label>
            <div className="artwork-colour">
              <input id="artwork-background" type="text" value={options.background} disabled={locked} spellCheck={false} autoComplete="off" maxLength={7}
                aria-invalid={colourInvalid} aria-describedby={colourInvalid ? 'artwork-background-error' : undefined} onChange={event => set('background', event.target.value)} />
              <input type="color" aria-label="Pick background colour" value={formatHexColour(colour ?? [0, 0, 0])} disabled={locked} onChange={event => set('background', event.target.value)} />
            </div>
            {colourInvalid ? <p id="artwork-background-error" className="field-error" role="alert">Background colour must be a hex value such as #000000 or #fff.</p>
              : <p className="fine-print">Fills margins and transparent areas.</p>}
          </div>
          <div className="artwork-field">
            <label htmlFor="artwork-quality">JPEG quality</label>
            <div className="position-inputs">
              <input id="artwork-quality" type="range" min={1} max={100} step={1} value={options.quality} disabled={locked} onChange={event => set('quality', clampQuality(Number(event.target.value)))} />
              <output htmlFor="artwork-quality">{options.quality}</output>
            </div>
            <p className="fine-print" aria-live="polite">{result && !loading ? `Prepared size: ${formatBytes(result.bytes)}` : 'Prepared size: —'}</p>
          </div>
        </section>
      </div>
      <section className="panel artwork-preview" aria-labelledby="artwork-preview-title">
        <div className="section-heading"><h2 id="artwork-preview-title">Preview</h2>
          {loading && <span className="small-label" role="status"><LoaderCircle size={14} className="spinning" /> Preparing preview…</span>}</div>
        {previewError && <div className="service-failure" role="alert"><strong>Could not prepare the image.</strong> {previewError}</div>}
        {!source && <p className="fine-print">Choose an image to see how it will look on the display.</p>}
        {source && !desktop && <p className="fine-print">The prepared preview needs the desktop app.</p>}
        {(source || result) && <div className="artwork-previews">
          <ArtworkStage target={options.target} src={previewSrc} busy={loading} />
          <figure className="artwork-raw">
            <div className="artwork-raw-frame" style={{ width: size, height: size }}>
              {previewSrc ? <img src={previewSrc} width={size} height={size} alt={`Prepared ${size} by ${size} image at actual size`} /> : <span className="artwork-display-empty" aria-hidden="true" />}
            </div>
            <figcaption className="fine-print">Prepared image, actual size {size}×{size}{result ? ` · ${formatBytes(result.bytes)} JPEG` : ''}</figcaption>
          </figure>
        </div>}
        <div className="artwork-export">
          <button type="button" className="button primary" disabled={locked || !request || !source || saving || loading || previewError !== null} onClick={() => { void exportJpeg(); }}>
            {saving ? <LoaderCircle size={16} className="spinning" /> : <FileDown size={16} />}{saving ? 'Exporting…' : 'Export JPEG'}
          </button>
          <span className="fine-print">Writes a new file to this app's artwork folder. The newest 50 are kept.</span>
        </div>
        {saveError && <div className="service-failure" role="alert"><strong>Could not export the JPEG.</strong> {saveError}</div>}
        {saved && <div className="diagnostics-result" role="status">
          <strong>JPEG exported</strong>
          <code className="storage-path">{saved.path}</code>
          <p className="fine-print">{saved.width}×{saved.height} · {formatBytes(saved.bytes)} · stays on this Mac. Nothing was sent to a device.</p>
          <div className="diagnostics-actions"><button type="button" className="button" onClick={() => { void copyText(saved.path, navigator.clipboard).then(setCopy); }}>{copy === 'Path copied.' ? <Check size={16} /> : <Copy size={16} />}Copy path</button>{copy && <span className="fine-print">{copy}</span>}</div>
        </div>}
      </section>
    </div>
  </section>;
}
