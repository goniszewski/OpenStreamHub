// The design of key artwork built from a bundled icon and a text label. The
// design is rendered on a canvas (KeyArtworkEditor) and prepared through the
// same backend command as uploaded images (prepare_key_artwork), so what the
// editor shows is the same JPEG the key receives. Nothing here talks to a
// device or the network.
import type { BindingAction, KeyArtworkDesignSpec } from './contracts';
import { parseHexColour, type PrepareRequest } from './artwork-model';
import { keyIconById } from './key-icons';

/** The app-icon suggestion uses the real .app icon, keyed by bundle id. */
export const appIconPrefix = 'app:';
export const appIconId = (bundleId: string) => `${appIconPrefix}${bundleId}`;
export const isAppIconId = (id: string) => id.startsWith(appIconPrefix);

/** Label text: optional (an icon alone is fine), otherwise the key label rules. */
export const designTextLimit = 128;

export function designTextIssue(text: string): string | null {
  if (new TextEncoder().encode(text).length > designTextLimit) return `Use at most ${designTextLimit} bytes.`;
  if (/[\u0000-\u001f\u007f-\u009f]/.test(text)) return 'Remove line breaks, tabs and other control characters.';
  return null;
}

/** Four text sizes, as offered by the official icon editor. */
export const keyTextSizes = [
  { label: 'Small', px: 11 },
  { label: 'Medium', px: 14 },
  { label: 'Large', px: 18 },
  { label: 'Extra large', px: 24 },
] as const;
export type KeyTextSize = 0 | 1 | 2 | 3;

/** Three icon sizes. Medium is the size icons were always drawn at; large fills the key edge to edge, with the label over it. */
export const keyIconSizes = [
  { label: 'Small', px: 28 },
  { label: 'Medium', px: 44 },
  { label: 'Large', px: 72 },
] as const;
export type KeyIconSize = 0 | 1 | 2;

export const keyTextAligns = ['top', 'middle', 'bottom'] as const;
export type KeyTextAlign = (typeof keyTextAligns)[number];
export const keyTextAlignLabels: Record<KeyTextAlign, string> = { top: 'Top', middle: 'Middle', bottom: 'Bottom' };

export interface KeyArtworkDesign {
  /** A catalog icon id, an `app:<bundle id>` suggestion, or null for text only. */
  iconId: string | null;
  text: string;
  textSize: KeyTextSize;
  iconSize: KeyIconSize;
  align: KeyTextAlign;
  /** As typed: `#rrggbb`, `rrggbb`, `#rgb` or `rgb`. */
  textColour: string;
  iconColour: string;
  background: string;
}

export const defaultKeyArtworkDesign: KeyArtworkDesign = {
  iconId: null, text: '', textSize: 1, iconSize: 1, align: 'bottom', textColour: '#ffffff', iconColour: '#ffffff', background: '#000000',
};

/** The design a studio opens with: the key's label as text and the first
    catalog suggestion for the bound action (the .app icon needs a lookup
    first, so it is offered as a chip instead of preselected). */
export function initialKeyArtworkDesign(action?: BindingAction | null, label = ''): KeyArtworkDesign {
  return { ...defaultKeyArtworkDesign, text: label, iconId: suggestedIconIds(action).find(id => !isAppIconId(id)) ?? null };
}

/** The stored form of a design, written beside the prepared frames when a
    design is applied. Colours are stored exactly as typed; applying is gated
    on valid hex. */
export function storedKeyArtworkDesign(design: KeyArtworkDesign): KeyArtworkDesignSpec {
  return {
    icon_id: design.iconId, text: design.text, text_size: design.textSize, icon_size: design.iconSize, align: design.align,
    text_colour: design.textColour, icon_colour: design.iconColour, background: design.background,
  };
}

/** Whether a stored icon id still names a bundled catalog entry or the bound
    application. A renamed entry or a different app degrades to the default
    suggestion instead of erroring. */
function storedIconResolves(iconId: string, action?: BindingAction | null): boolean {
  return keyIconById.has(iconId)
    || (isAppIconId(iconId) && action?.type === 'launch_app' && action.bundle_id === iconId.slice(appIconPrefix.length));
}

/** The studio design a saved artwork reopens with, or null when the artwork
    carries no design (an uploaded image). The backend validates the stored
    form, so a field that still cannot shape itself falls back to its default. */
export function keyDesignFromSpec(spec: KeyArtworkDesignSpec | undefined, action?: BindingAction | null): KeyArtworkDesign | null {
  if (!spec) return null;
  let iconId = spec.icon_id ?? null;
  if (iconId !== null && !storedIconResolves(iconId, action)) {
    iconId = suggestedIconIds(action).find(id => !isAppIconId(id)) ?? null;
  }
  const size = spec.text_size;
  const iconSize = spec.icon_size;
  return {
    iconId,
    text: designTextIssue(spec.text) === null ? spec.text : defaultKeyArtworkDesign.text,
    textSize: Number.isInteger(size) && size >= 0 && size < keyTextSizes.length ? size as KeyArtworkDesign['textSize'] : defaultKeyArtworkDesign.textSize,
    iconSize: Number.isInteger(iconSize) && iconSize! >= 0 && iconSize! < keyIconSizes.length ? iconSize as KeyIconSize : defaultKeyArtworkDesign.iconSize,
    align: (keyTextAligns as readonly string[]).includes(spec.align) ? spec.align : defaultKeyArtworkDesign.align,
    textColour: parseHexColour(spec.text_colour) !== null ? spec.text_colour : defaultKeyArtworkDesign.textColour,
    iconColour: parseHexColour(spec.icon_colour) !== null ? spec.icon_colour : defaultKeyArtworkDesign.iconColour,
    background: parseHexColour(spec.background) !== null ? spec.background : defaultKeyArtworkDesign.background,
  };
}

// Geometry of the 72×72 key: content padding, the icon/text gap and the largest
// icon. The canvas renderer draws exactly what this layout computes.
export const keyCanvasSize = 72;
export const keyPadding = 6;
const keyGap = 3;
const lineHeightFactor = 1.2;

/** Line counts fall as the text grows, so large text never wraps forever. */
export const keyTextMaxLines = (px: number): number => (px <= keyTextSizes[1].px ? 3 : px <= keyTextSizes[2].px ? 2 : 1);

/** Width of `text` at a size, so layout is testable without a canvas. */
export type KeyTextMeasure = (text: string, px: number) => number;

export interface KeyDesignLayout { icon: { x: number; y: number; size: number } | null; lines: { text: string; y: number }[] }

/** Greedy word wrap. A word wider than a line is hard-broken, and dropped text
    ends the last visible line in an ellipsis. */
export function wrapKeyText(text: string, px: number, maxLines: number, measure: KeyTextMeasure): string[] {
  const maxWidth = keyCanvasSize - 2 * keyPadding;
  const width = (line: string) => measure(line, px);
  const atoms: string[] = [];
  for (const word of text.split(/\s+/).filter(Boolean)) {
    let rest = word;
    while (rest && width(rest) > maxWidth && rest.length > 1) {
      let cut = rest.length - 1;
      while (cut > 1 && width(rest.slice(0, cut)) > maxWidth) cut--;
      atoms.push(rest.slice(0, cut));
      rest = rest.slice(cut);
    }
    if (rest) atoms.push(rest);
  }
  const lines: string[] = [];
  for (const atom of atoms) {
    const last = lines.at(-1);
    if (last === undefined) lines.push(atom);
    else if (width(`${last} ${atom}`) <= maxWidth) lines[lines.length - 1] = `${last} ${atom}`;
    else lines.push(atom);
  }
  if (lines.length <= maxLines) return lines;
  let last = lines[maxLines - 1];
  while (last.length > 1 && width(`${last}…`) > maxWidth) last = last.slice(0, -1);
  return [...lines.slice(0, maxLines - 1), `${last}…`];
}

/** Where the icon and each text line (top edges) sit on the 72×72 canvas. */
export function keyDesignLayout(design: KeyArtworkDesign, measure: KeyTextMeasure): KeyDesignLayout {
  const maxIconSize = keyIconSizes[design.iconSize].px;
  const fullBleed = maxIconSize >= keyCanvasSize;
  const px = keyTextSizes[design.textSize].px;
  const lineHeight = px * lineHeightFactor;
  const lines = wrapKeyText(design.text.trim(), px, keyTextMaxLines(px), measure);
  if (!lines.length) {
    return { icon: design.iconId ? { x: (keyCanvasSize - maxIconSize) / 2, y: (keyCanvasSize - maxIconSize) / 2, size: maxIconSize } : null, lines: [] };
  }
  const textHeight = lines.length * lineHeight;
  const textTop = design.align === 'top' ? keyPadding
    : design.align === 'bottom' ? keyCanvasSize - keyPadding - textHeight
    : (keyCanvasSize - textHeight) / 2;
  if (!design.iconId) return { icon: null, lines: lines.map((text, index) => ({ text, y: textTop + index * lineHeight })) };
  if (fullBleed) return { icon: { x: 0, y: 0, size: keyCanvasSize }, lines: lines.map((text, index) => ({ text, y: textTop + index * lineHeight })) };
  const band = design.align === 'top'
    ? { from: textTop + textHeight + keyGap, to: keyCanvasSize - keyPadding }
    : { from: keyPadding, to: textTop - keyGap };
  const iconSize = Math.min(maxIconSize, keyCanvasSize - 2 * keyPadding, band.to - band.from);
  return {
    icon: iconSize > 0 ? { x: (keyCanvasSize - iconSize) / 2, y: (band.from + band.to - iconSize) / 2, size: iconSize } : null,
    lines: lines.map((text, index) => ({ text, y: textTop + index * lineHeight })),
  };
}

// Which bundled icons fit the assigned action: the first is the one the studio
// preselects, the rest are one click away in the suggested row. The launch_app
// entry carries a bare 'app:' placeholder that gains the action's bundle id.
const actionSuggestions: Partial<Record<BindingAction['type'], string[]>> = {
  volume: ['volume-2', 'volume-1'], set_volume: ['volume-2', 'volume-1'], volume_from_position: ['volume-2', 'volume-1'], mic_input_volume_from_position: ['mic'], obs_input_volume_from_position: ['sliders-vertical'], obs_set_input_volume: ['sliders-vertical'],
  toggle_mute: ['volume-x', 'mic-off'],
  shortcut: ['keyboard'],
  launch_app: [appIconId(''), 'app-window'],
  open_url: ['globe', 'link'],
  text: ['type'],
  media: ['play', 'pause'],
  system: ['undo-2', 'camera'],
  multi_action: ['list-ordered'],
  profile: ['arrow-left-right'],
  timer: ['timer', 'alarm-clock'],
  obs_scene: ['clapperboard'], obs_virtual_camera: ['video', 'video-off'], obs_replay_buffer: ['film'], obs_stream: ['radio', 'cast'], obs_screenshot: ['camera'], obs_input_mute: ['mic-off', 'volume-x'], obs_adjust_input_volume: ['volume-2'], obs_media_source: ['play', 'pause'], obs_record: ['circle', 'pause'],
  obs_studio_mode: ['columns-2'], obs_transition: ['film', 'arrow-left-right'], obs_filter: ['sparkles'],
  obs_source_visibility: ['eye', 'eye-off'], obs_scene_collection: ['layers'], obs_profile: ['arrow-left-right'],
};
const commandSuggestions: Record<string, string[]> = {
  play_pause: ['play', 'pause'], next_track: ['skip-forward'], previous_track: ['skip-back'], stop: ['square'],
  undo: ['undo-2'], redo: ['redo-2'], screenshot: ['camera'],
};

/** Suggested icon ids for an action, empty when nothing fits. */
export function suggestedIconIds(action?: BindingAction | null): string[] {
  if (!action) return [];
  const byCommand = 'command' in action ? commandSuggestions[action.command] : undefined;
  return (byCommand ?? actionSuggestions[action.type] ?? [])
    .map(id => isAppIconId(id) && 'bundle_id' in action ? appIconId(action.bundle_id) : id);
}

/** The prepared-image request for a rendered design: identity geometry, since
    the canvas is already exactly the key's 72×72 pixels. Only the background
    colour travels to the backend — text and icon colours are baked into the
    rendered pixels. */
export function buildKeyArtworkDesignRequest(design: KeyArtworkDesign, pngBase64: string): { ok: true; request: PrepareRequest } | { ok: false; error: string } {
  if (!pngBase64) return { ok: false, error: 'Render the design first.' };
  for (const [what, colour] of [['Background', design.background], ['Text', design.textColour], ['Icon', design.iconColour]] as const) {
    if (!parseHexColour(colour)) return { ok: false, error: `${what} colour must be a hex value such as #000000 or #fff.` };
  }
  return {
    ok: true,
    request: {
      image_base64: pngBase64, target: 'keypad', quality: 85, fit: 'contain', quarter_turns: 0,
      background: parseHexColour(design.background)!,
    },
  };
}

// Recently used icons, newest first, shown at the top of the library. Like the
// action picker's recents: unreadable or missing storage yields an empty list
// and a failed write is dropped.
export type DesignStorage = Pick<Storage, 'getItem' | 'setItem'>;

export const recentKeyIconsStorageKey = 'open-stream-hub.recent-key-icons';
export const recentKeyIconsLimit = 8;

export function readRecentKeyIcons(storage: DesignStorage): string[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(recentKeyIconsStorageKey) ?? 'null');
    if (!Array.isArray(parsed)) return [];
    return [...new Set(parsed)].filter((item): item is string => typeof item === 'string' && (keyIconById.has(item) || isAppIconId(item)));
  } catch { return []; }
}

export function recordRecentKeyIcon(storage: DesignStorage, id: string): void {
  try {
    storage.setItem(recentKeyIconsStorageKey, JSON.stringify([id, ...readRecentKeyIcons(storage).filter(item => item !== id)].slice(0, recentKeyIconsLimit)));
  } catch { /* The recent list must never block applying artwork. */ }
}

/** Catalog icon ids matching a search across id, name and keywords; all ids in
    catalog order for an empty query. */
export function searchKeyIcons(query: string): string[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [...keyIconById.keys()];
  return [...keyIconById.values()]
    .filter(icon => `${icon.id} ${icon.name} ${icon.keywords}`.toLowerCase().includes(needle))
    .map(icon => icon.id);
}
