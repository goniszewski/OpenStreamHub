import { ColourSwatchPicker } from './ColourPicker';
import { useEffect, useRef, useState } from 'react';
import type { BindingAction, KeyArtwork } from './contracts';
import { dataUrl, formatHexColour, parseHexColour, readSourceFile } from './artwork-model';
import { keyArtworkFrame, setKeyArtwork } from './key-artwork';
import {
  appIconPrefix, buildKeyArtworkDesignRequest, designTextIssue, initialKeyArtworkDesign, isAppIconId,
  keyCanvasSize, keyDesignFromSpec, keyTextAlignLabels, keyTextAligns, keyIconSizes, keyTextSizes, keyDesignLayout,
  readRecentKeyIcons, recordRecentKeyIcon, searchKeyIcons, storedKeyArtworkDesign, suggestedIconIds,
} from './key-artwork-design';
import type { DesignStorage, KeyArtworkDesign, KeyTextMeasure } from './key-artwork-design';
import { stableStringify } from './draft-model';
import { keyIconById } from './key-icons';
import { applyAutoArtwork, planAutoArtwork, planKey } from './auto-key-artwork';
import type { Invoke, ProfileSession } from './profile-session';
import { artworkErrorNotice } from './artwork-error-model';
import { ArtworkErrorNote } from './ArtworkErrorNote';
import { errorMessage } from './ui-model';
import type { ArtworkNotice } from './artwork-error-model';

export interface AppIconResult { png_base64: string }

// The app icon of one bundle id costs one LaunchServices lookup; the editor
// remounts on every profile edit, so the result is cached module-wide.
const appIconCache = new Map<string, string>();

// Typing a bundle id must settle before the automatic drawing looks it up.
export const autoArtworkDelayMs = 500;

/** The icon of an installed application, cached for the session. */
async function loadAppIcon(invoke: Invoke, bundleId: string): Promise<string> {
  const cached = appIconCache.get(bundleId);
  if (cached) return cached;
  const result = await invoke<AppIconResult>('app_icon', { request: { bundle_id: bundleId } });
  appIconCache.set(bundleId, result.png_base64);
  return result.png_base64;
}

// Recently used icons live beside the action picker's recents. A failed read
// or write must never block assigning artwork.
const keyDesignStorage: DesignStorage = {
  getItem: key => globalThis.localStorage?.getItem(key) ?? null,
  setItem: (key, value) => globalThis.localStorage?.setItem(key, value),
};

// The key canvas draws the same design the preview shows; the system font is
// used so nothing is fetched and the raster matches what was measured.
const keyFontStack = '-apple-system, system-ui, sans-serif';

const svgMarkup = (inner: string, stroke: string) => `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="${stroke}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;

const loadImage = (src: string) => new Promise<HTMLImageElement>((resolve, reject) => {
  const image = new Image();
  image.onload = () => resolve(image);
  image.onerror = () => reject(new Error('The icon could not be drawn.'));
  image.src = src;
});

/** Paints the design on a prepared 72×72 canvas: background, icon, text. */
async function drawKeyDesign(context: CanvasRenderingContext2D, design: KeyArtworkDesign, appIconPng?: string): Promise<void> {
  const measure: KeyTextMeasure = (text, px) => {
    context.font = `600 ${px}px ${keyFontStack}`;
    return context.measureText(text).width;
  };
  context.fillStyle = design.background;
  context.fillRect(0, 0, keyCanvasSize, keyCanvasSize);
  const layout = keyDesignLayout(design, measure);
  if (layout.icon && design.iconId) {
    const source = isAppIconId(design.iconId)
      ? appIconPng ? await loadImage(dataUrl('image/png', appIconPng)) : null
      : await loadImage(`data:image/svg+xml;utf8,${encodeURIComponent(svgMarkup(keyIconById.get(design.iconId)?.svg ?? '', design.iconColour))}`);
    if (source) context.drawImage(source, layout.icon.x, layout.icon.y, layout.icon.size, layout.icon.size);
  }
  context.font = `600 ${keyTextSizes[design.textSize].px}px ${keyFontStack}`;
  context.fillStyle = design.textColour;
  context.textAlign = 'center';
  context.textBaseline = 'top';
  for (const line of layout.lines) context.fillText(line.text, keyCanvasSize / 2, line.y);
}

/** The design as PNG bytes (base64), exactly as painted on the preview. */
export async function renderKeyDesign(design: KeyArtworkDesign, appIconPng?: string): Promise<string> {
  const canvas = document.createElement('canvas');
  canvas.width = keyCanvasSize;
  canvas.height = keyCanvasSize;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('The canvas is unavailable.');
  await drawKeyDesign(context, design, appIconPng);
  return canvas.toDataURL('image/png').split(',')[1] ?? '';
}

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

// The live canvas of the studio. It is a separate hooking component so the
// studio itself stays hook-free and testable by function call.
function KeyDesignCanvasPreview({ design, appIconPng }: { design: KeyArtworkDesign; appIconPng?: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    element.width = keyCanvasSize;
    element.height = keyCanvasSize;
    const context = element.getContext('2d');
    if (!context) return;
    void drawKeyDesign(context, design, appIconPng).catch(() => { /* the last painted frame stays */ });
  }, [design, appIconPng]);
  return <canvas ref={canvas} width={keyCanvasSize} height={keyCanvasSize} className="key-artwork-image key-design-preview" role="img" aria-label="Key artwork preview" />;
}

const colourRow = (id: string, label: string, value: string, disabled: boolean, onChange: (value: string) => void) => <div className="artwork-field">
  <label htmlFor={id}>{label}</label>
  <div className="artwork-colour">
    <input id={id} type="text" value={value} disabled={disabled} spellCheck={false} autoComplete="off" maxLength={7}
      aria-invalid={parseHexColour(value) === null} onChange={event => onChange(event.target.value)} />
    <ColourSwatchPicker label={`Pick ${label.toLowerCase()}`} value={formatHexColour(parseHexColour(value) ?? [0, 0, 0])} disabled={disabled} onChange={onChange} />
  </div>
</div>;

export interface KeyDesignStudioProps {
  design: KeyArtworkDesign;
  onDesign(design: KeyArtworkDesign): void;
  /** The action bound to the key, which drives the suggested icons. */
  action?: BindingAction | null;
  /** The fetched .app icon for a launch_app action, when it loaded. */
  appIcon: { bundleId: string; pngBase64: string } | null;
  appIconFailed: boolean;
  recents: string[];
  query: string;
  onQuery(query: string): void;
  libraryOpen: boolean;
  onLibraryOpen(open: boolean): void;
  preview: React.ReactNode;
  /** The studio differs from what the key currently shows: the apply bar stays in view and says so. */
  dirty?: boolean;
  disabled: boolean;
  busy: boolean;
  prepareAvailable: boolean;
  onApply(): void;
}

const designColourIssue = (design: KeyArtworkDesign): string | null => {
  for (const [what, colour] of [['Background', design.background], ['Text', design.textColour], ['Icon', design.iconColour]] as const) {
    if (!parseHexColour(colour)) return `${what} colour must be a hex value such as #000000 or #fff.`;
  }
  return null;
};

/** The icon-and-label studio shown above the file picker in the key artwork
    editor. Hook-free and controlled: all state lives in KeyArtworkEditor, so
    tests call this directly and walk the element tree. */
export function KeyDesignStudio({ design, onDesign, action, appIcon, appIconFailed, recents, query, onQuery, libraryOpen, onLibraryOpen, preview, dirty = false, disabled, busy, prepareAvailable, onApply }: KeyDesignStudioProps) {
  const set = <K extends keyof KeyArtworkDesign>(key: K, value: KeyArtworkDesign[K]) => onDesign({ ...design, [key]: value });
  const issue = designTextIssue(design.text) ?? designColourIssue(design);
  const suggestions = suggestedIconIds(action).filter(id => !isAppIconId(id) || (appIcon && appIcon.bundleId === id.slice(appIconPrefix.length)));
  const matches = searchKeyIcons(query);
  const iconButton = (id: string, name: string) => <button key={id} type="button" className={`key-icon${design.iconId === id ? ' active' : ''}`}
    aria-pressed={design.iconId === id} aria-label={name} disabled={disabled || busy}
    onClick={() => set('iconId', design.iconId === id ? null : id)}
    dangerouslySetInnerHTML={{ __html: svgMarkup(keyIconById.get(id)?.svg ?? '', 'currentColor') }} />;
  const appSuggestion = action?.type === 'launch_app' && !appIconFailed
    ? <button type="button" className={`key-icon key-icon-app${design.iconId === appIconPrefix + action.bundle_id ? ' active' : ''}`}
      aria-pressed={design.iconId === appIconPrefix + action.bundle_id}
      aria-label="Application icon" disabled={disabled || busy || !appIcon}
      title={appIcon ? 'The application’s own icon' : appIconFailed ? undefined : 'Loading the application icon…'}
      onClick={() => appIcon && set('iconId', appIconPrefix + action.bundle_id)}>{appIcon ? <img src={dataUrl('image/png', appIcon.pngBase64)} alt="" width={22} height={22} /> : <span className="key-icon-loading" aria-hidden="true" />}</button>
    : null;
  return <div className="key-design-studio">
    <div className="key-design-head"><h3>Icon and label</h3>{preview}</div>
    {(suggestions.length > 0 || appSuggestion) && <div className="key-design-suggested" role="group" aria-label="Suggested icons">
      <span className="picker-group-label">Suggested</span>
      {appSuggestion}
      {suggestions.map(id => iconButton(id, keyIconById.get(id)?.name ?? id))}
    </div>}
    <details className="key-icon-library" open={libraryOpen} onToggle={event => onLibraryOpen((event.target as HTMLDetailsElement).open)}>
      <summary>Icon library</summary>
      <input type="search" id="key-icon-search" placeholder="Search icons" value={query} disabled={disabled || busy}
        aria-label="Search icons" onChange={event => onQuery(event.target.value)} />
      {!query && recents.length > 0 && <><p className="picker-group-label">Recently used</p><div className="key-icon-grid" role="group" aria-label="Recently used icons">
        {recents.filter(id => keyIconById.has(id)).map(id => iconButton(id, keyIconById.get(id)?.name ?? id))}
      </div></>}
      {matches.length ? <><p className="picker-group-label">{query ? 'Results' : 'All icons'}</p><div className="key-icon-grid" role="list" aria-label="Icons">
        {matches.map(id => iconButton(id, keyIconById.get(id)?.name ?? id))}
      </div></> : <p className="picker-empty">No icon matches.</p>}
    </details>
    <div className="artwork-field">
      <label htmlFor="key-design-text">Label text</label>
      <input id="key-design-text" type="text" value={design.text} disabled={disabled || busy} maxLength={128}
        aria-invalid={designTextIssue(design.text) !== null} onChange={event => set('text', event.target.value)} />
    </div>
    {design.iconId && <div className="artwork-field">
      <span className="picker-group-label">Icon size</span>
      <div className="artwork-seg" role="group" aria-label="Icon size">
        {keyIconSizes.map((size, index) => <button key={size.label} type="button" aria-pressed={design.iconSize === index}
          className={design.iconSize === index ? 'active' : ''} disabled={disabled || busy}
          onClick={() => set('iconSize', index as KeyArtworkDesign['iconSize'])}>{size.label}</button>)}
      </div>
    </div>}
    <div className="artwork-field">
      <span className="picker-group-label">Text size</span>
      <div className="artwork-seg" role="group" aria-label="Text size">
        {keyTextSizes.map((size, index) => <button key={size.label} type="button" aria-pressed={design.textSize === index}
          className={design.textSize === index ? 'active' : ''} disabled={disabled || busy}
          onClick={() => set('textSize', index as KeyArtworkDesign['textSize'])}>{size.label}</button>)}
      </div>
    </div>
    <div className="artwork-field">
      <span className="picker-group-label">Text alignment</span>
      <div className="artwork-seg" role="group" aria-label="Text alignment">
        {keyTextAligns.map(align => <button key={align} type="button" aria-pressed={design.align === align}
          className={design.align === align ? 'active' : ''} disabled={disabled || busy}
          onClick={() => set('align', align)}>{keyTextAlignLabels[align]}</button>)}
      </div>
    </div>
    {colourRow('key-design-text-colour', 'Text colour', design.textColour, disabled || busy, value => set('textColour', value))}
    {colourRow('key-design-icon-colour', 'Icon colour', design.iconColour, disabled || busy, value => set('iconColour', value))}
    {colourRow('key-design-background', 'Background colour', design.background, disabled || busy, value => set('background', value))}
    <div className={`key-design-apply${dirty ? ' dirty' : ''}`}>
      {dirty && !busy && <p className="key-design-unsaved" role="status">Not applied yet. The key still shows its previous artwork.</p>}
      <button type="button" className="button primary compact" disabled={disabled || busy || !prepareAvailable || issue !== null
        || (isAppIconId(design.iconId ?? '') && (!appIcon || appIcon.bundleId !== (design.iconId ?? '').slice(appIconPrefix.length)))}
        onClick={onApply}>{busy ? 'Preparing…' : 'Set key artwork'}</button>
      {issue && <p className="field-error" role="alert">{issue}</p>}
    </div>
  </div>;
}

export function KeyArtworkEditor({ session, path, control, artwork, action, initialText, disabled, invoke }: {
  session: ProfileSession; path: string[]; control: number; artwork?: KeyArtwork; action?: BindingAction | null;
  /** The key's current label, offered as the starting point for the label text. */
  initialText?: string; disabled: boolean; invoke?: Invoke;
}) {
  // A key with a saved design reopens the studio with it; without one the
  // studio starts from the key's label and the action's suggestion.
  const [design, setDesign] = useState<KeyArtworkDesign>(() =>
    keyDesignFromSpec(artwork?.design, action) ?? initialKeyArtworkDesign(action, initialText ?? ''));
  // Until the user touches the studio, a key without a saved design follows
  // the key: an action assigned after selection (the editor does not remount
  // for ordinary edits) still preselects its suggestion, and a renamed label
  // updates the text. A saved design is sticky — it survives label and action
  // edits, and removing the artwork returns the studio to the fresh state.
  const touched = useRef(false);
  const editDesign = (next: KeyArtworkDesign) => { touched.current = true; setDesign(next); };
  useEffect(() => {
    if (touched.current) return;
    // An automatic drawing lands as saved artwork with a design: the studio follows it.
    setDesign((artwork?.design && keyDesignFromSpec(artwork.design, action)) || initialKeyArtworkDesign(action, initialText ?? ''));
  }, [action, initialText, artwork]);
  // Whether the studio shows something the key does not: unlike an upload, a design only reaches the key when applied.
  const saved = artwork?.design ? keyDesignFromSpec(artwork.design, action) : null;
  const dirty = saved ? stableStringify(storedKeyArtworkDesign(design)) !== stableStringify(storedKeyArtworkDesign(saved))
    : touched.current && stableStringify(design) !== stableStringify(initialKeyArtworkDesign(action, initialText ?? ''));
  const [query, setQuery] = useState('');
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [recents, setRecents] = useState(() => readRecentKeyIcons(keyDesignStorage));
  const [appIcon, setAppIcon] = useState<{ bundleId: string; pngBase64: string } | null>(() => {
    const bundleId = action?.type === 'launch_app' ? action.bundle_id : null;
    const cached = bundleId ? appIconCache.get(bundleId) : undefined;
    return cached ? { bundleId: bundleId!, pngBase64: cached } : null;
  });
  const [appIconFailed, setAppIconFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ArtworkNotice | null>(null);
  const requestId = useRef(0);
  // Which prepare call owns the "Preparing" state. Kept apart from requestId: the
  // auto-artwork timer bumps requestId to discard an upload still in flight, and
  // that upload's own finish must still clear busy.
  const busyId = useRef(0);
  useEffect(() => () => { requestId.current++; }, []);
  // The real .app icon for a launch_app action, asked once per bundle id.
  useEffect(() => {
    if (action?.type !== 'launch_app' || !invoke) return;
    const bundleId = action.bundle_id;
    if (appIconCache.has(bundleId)) return;
    let cancelled = false;
    invoke<AppIconResult>('app_icon', { request: { bundle_id: bundleId } })
      .then(result => {
        appIconCache.set(bundleId, result.png_base64);
        if (!cancelled) setAppIcon({ bundleId, pngBase64: result.png_base64 });
      })
      .catch(() => { if (!cancelled) setAppIconFailed(true); });
    return () => { cancelled = true; };
  }, [action, invoke]);
  // Assigning an application or a URL draws the key's artwork (see
  // auto-key-artwork); only an assignment made while this editor is open does,
  // never merely selecting a key. Removing the artwork declines it for that
  // assignment, and anything the user designed is never replaced.
  const plan = planAutoArtwork(action);
  const planId = planKey(plan);
  const latest = useRef({ planId, artwork });
  latest.current = { planId, artwork };
  const previousPlan = useRef(planId);
  const declined = useRef<string | null>(null);
  useEffect(() => {
    if (previousPlan.current === planId) return;
    previousPlan.current = planId;
    if (!plan || !invoke || disabled || declined.current === planId) return;
    // The application field is typed into: settle before looking anything up.
    const timer = setTimeout(() => {
      const id = ++requestId.current;
      void applyAutoArtwork(plan, { invoke, render: renderKeyDesign, appIcon: bundleId => loadAppIcon(invoke, bundleId) }, session, path, control, {
        planKey: () => latest.current.planId, artwork: () => latest.current.artwork, current: () => id === requestId.current,
      });
    }, autoArtworkDelayMs);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [planId]);
  async function choose(file: File) {
    const id = ++requestId.current;
    const busyToken = ++busyId.current;
    const before = session.getState();
    setBusy(true); setError(null);
    try {
      const source = await readSourceFile(file);
      if (!source.ok) { if (id === requestId.current) setError({ message: source.error }); return; }
      const prepared = await invoke!<KeyArtwork>('prepare_key_artwork', { request: {
        image_base64: source.base64, target: 'keypad', quality: 85, fit: 'contain', quarter_turns: 0, background: [0, 0, 0],
      } });
      if (id !== requestId.current) return;
      // Never apply completion to a different draft, nor undo intervening edits.
      const current = session.getState();
      if (current.draftGeneration !== before.draftGeneration || current.draft.profile !== before.draft.profile) {
        setError({ message: 'The profile changed while preparing artwork. Choose the file again.' }); return;
      }
      // The prepared artwork carries no design, so an upload clears a stored
      // one: the file, not the studio, is now what the key shows.
      session.edit(setKeyArtwork(current.draft.profile, path, control, prepared));
    } catch (reason) { if (id === requestId.current) setError(artworkErrorNotice(reason)); }
    finally { if (busyToken === busyId.current) setBusy(false); }
  }
  async function applyDesign() {
    const id = ++requestId.current;
    const busyToken = ++busyId.current;
    const before = session.getState();
    setBusy(true); setError(null);
    try {
      let png: string;
      try { png = await renderKeyDesign(design, appIcon?.pngBase64); } catch (reason) {
        // The studio's own drawing errors are already plain language.
        if (id === requestId.current) setError({ message: errorMessage(reason) });
        return;
      }
      const built = buildKeyArtworkDesignRequest(design, png);
      if (!built.ok) { if (id === requestId.current) setError({ message: built.error }); return; }
      const prepared = await invoke!<KeyArtwork>('prepare_key_artwork', { request: built.request });
      if (id !== requestId.current) return;
      const current = session.getState();
      if (current.draftGeneration !== before.draftGeneration || current.draft.profile !== before.draft.profile) {
        setError({ message: 'The profile changed while preparing artwork. Apply again.' }); return;
      }
      // Store the design beside the frames so reopening restores the studio.
      // An uploaded file (choose) prepares design-less artwork, which clears it.
      session.edit(setKeyArtwork(current.draft.profile, path, control, { ...prepared, design: storedKeyArtworkDesign(design) }));
      if (design.iconId) {
        recordRecentKeyIcon(keyDesignStorage, design.iconId);
        setRecents(readRecentKeyIcons(keyDesignStorage));
      }
    } catch (reason) { if (id === requestId.current) setError(artworkErrorNotice(reason)); }
    finally { if (busyToken === busyId.current) setBusy(false); }
  }
  return <section className="key-artwork-editor" aria-label="Key artwork">
    <KeyDesignStudio design={design} onDesign={editDesign} action={action} appIcon={appIcon} appIconFailed={appIconFailed}
      recents={recents} query={query} onQuery={setQuery} libraryOpen={libraryOpen} onLibraryOpen={setLibraryOpen}
      preview={<KeyDesignCanvasPreview design={design} appIconPng={appIcon?.pngBase64} />} dirty={dirty}
      disabled={disabled} busy={busy} prepareAvailable={!!invoke} onApply={() => { void applyDesign(); }} />
    <div className="inspector-divider" />
    <label className="key-artwork-upload-title" htmlFor="key-artwork-file">Image or GIF file</label>
    <input id="key-artwork-file" type="file" accept="image/png,image/jpeg,image/gif,image/bmp,.png,.jpg,.jpeg,.gif,.bmp" disabled={disabled || busy || !invoke} onChange={event => {
      const file = event.target.files?.[0]; event.target.value = ''; if (file) void choose(file);
    }} />
    {busy && <p role="status">Preparing key artwork…</p>}
    {error && <ArtworkErrorNote notice={error} />}
    {artwork && <><KeyArtworkPreview artwork={artwork} /><p className="fine-print">{artwork.frames.length > 1 ? `${artwork.frames.length} frames · ${artwork.loop_count === 0 ? 'loops continuously' : `${artwork.loop_count} play(s)`}` : 'Static image'}</p><button className="button compact" disabled={disabled || busy} onClick={() => { declined.current = planId; session.edit(setKeyArtwork(session.getState().draft.profile, path, control, null)); }}>Remove artwork</button></>}
    <p className="fine-print">The preview above shows the design; the key receives the same picture prepared as a 72×72 JPEG. PNG, JPEG or GIF · up to 8 MiB. GIFs: up to 512×512 and 120 frames, at most 10 FPS. Artwork is saved with the profile and sent while OpenStreamHub drives the display.</p>
    {!invoke && <p className="fine-print">Preparing key artwork needs the desktop app.</p>}
  </section>;
}
