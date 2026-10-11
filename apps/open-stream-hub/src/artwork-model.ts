// Offline display artwork: request building and validation, base64 helpers,
// and a latest-wins runner. Nothing here talks to a device or the network;
// the backend commands only prepare an image and write it to a local folder.
import { errorMessage } from './ui-model';
import type { Invoke } from './profile-session';

import type { LightingModule } from './lighting-model';

export type ArtworkTarget = 'dial' | 'keypad';
export type ArtworkFit = 'contain' | 'cover' | 'stretch';
export type Rgb = [number, number, number];

export const maxSourceBytes = 8 * 1024 * 1024;
export const debounceMs = 250;

export const targetInfo: Record<ArtworkTarget, { label: string; size: number }> = {
  dial: { label: 'Encoder dial', size: 240 },
  keypad: { label: 'Key', size: 72 },
};

// Which artwork target a picked module takes: the encoder dial's round screen
// or a 15-key module's key screens. Other modules have no display.
export function artworkTargetFor(module: LightingModule): ArtworkTarget | null {
  return module === 'm131e' ? 'dial' : module === 'm4315' ? 'keypad' : null;
}

export const moduleAcceptsDisplayArtwork = (device: { module: LightingModule }): boolean => artworkTargetFor(device.module) !== null;

export function moduleArtworkCaption(device: { module: LightingModule }): string {
  const target = artworkTargetFor(device.module);
  return target === 'dial' ? 'Round display, 240 px' : target === 'keypad' ? 'Key displays, 72 px' : 'No display';
}

export const fitInfo: Record<ArtworkFit, { label: string; help: string }> = {
  contain: { label: 'Contain', help: 'Keep the whole image and fill the margins with the background colour.' },
  cover: { label: 'Cover', help: 'Crop a centred square so the image fills the display.' },
  stretch: { label: 'Stretch', help: 'Scale width and height independently; the picture may distort.' },
};

export const rotations = [0, 1, 2, 3] as const;
export const rotationLabel = (quarterTurns: number) => `${clampQuarterTurns(quarterTurns) * 90}°`;

export interface ArtworkOptions {
  target: ArtworkTarget;
  fit: ArtworkFit;
  quarterTurns: number;
  /** As typed: `#rrggbb`, `rrggbb`, `#rgb` or `rgb`. */
  background: string;
}

export const defaultArtworkOptions: ArtworkOptions = { target: 'dial', fit: 'contain', quarterTurns: 0, background: '#000000' };

/** Where preparation starts; the backend lowers it itself when a frame must fit a size budget. */
export const preparedQuality = 100;

export interface PrepareRequest {
  image_base64: string;
  target: ArtworkTarget;
  quality: number;
  fit: ArtworkFit;
  quarter_turns: number;
  background: Rgb;
}
export interface SaveRequest extends PrepareRequest { name: string }
export interface PrepareResult { jpeg_base64: string; width: number; height: number; bytes: number }
export interface SaveResult { path: string; bytes: number; width: number; height: number }

export const clampQuarterTurns = (value: number) => Number.isFinite(value) ? ((Math.round(value) % 4) + 4) % 4 : 0;

export function parseHexColour(text: string): Rgb | null {
  const match = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(text.trim());
  if (!match) return null;
  const digits = match[1].length === 3 ? match[1].replace(/./g, digit => digit + digit) : match[1];
  return [0, 2, 4].map(start => parseInt(digits.slice(start, start + 2), 16)) as Rgb;
}

export const formatHexColour = (rgb: Rgb) => `#${rgb.map(channel => Math.min(255, Math.max(0, Math.round(channel))).toString(16).padStart(2, '0')).join('')}`;

export type RequestOutcome = { ok: true; request: PrepareRequest } | { ok: false; error: string };

export function buildArtworkRequest(imageBase64: string | null, options: ArtworkOptions): RequestOutcome {
  if (!imageBase64) return { ok: false, error: 'Choose a PNG or JPEG image first.' };
  const background = parseHexColour(options.background);
  if (!background) return { ok: false, error: 'Background colour must be a hex value such as #000000 or #fff.' };
  return {
    ok: true,
    request: {
      image_base64: imageBase64, target: options.target, quality: preparedQuality, fit: options.fit,
      quarter_turns: clampQuarterTurns(options.quarterTurns), background,
    },
  };
}

export const buildSaveRequest = (request: PrepareRequest, name: string): SaveRequest => ({ ...request, name: name.trim() });

/** File name without its extension, used as the saved file's name seed. */
export const artworkName = (fileName: string) => fileName.replace(/\.[^.]*$/, '').trim() || 'artwork';

export function bytesToBase64(bytes: Uint8Array): string {
  const chunk = 0x8000;
  const parts: string[] = [];
  for (let start = 0; start < bytes.length; start += chunk) parts.push(String.fromCharCode(...bytes.subarray(start, start + chunk)));
  return btoa(parts.join(''));
}

export type SourceOutcome = { ok: true; base64: string; mime: string; size: number } | { ok: false; error: string };

export async function readSourceFile(file: Pick<Blob, 'size' | 'arrayBuffer'> & { type?: string }): Promise<SourceOutcome> {
  if (file.size === 0) return { ok: false, error: 'The file is empty.' };
  if (file.size > maxSourceBytes) return { ok: false, error: 'The image is larger than 8 MiB.' };
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    return { ok: true, base64: bytesToBase64(bytes), mime: file.type || 'image/png', size: bytes.length };
  } catch (reason) { return { ok: false, error: `Could not read the file: ${errorMessage(reason)}` }; }
}

export const dataUrl = (mime: string, base64: string) => `data:${mime};base64,${base64}`;

export type Outcome<T> = { ok: true; value: T } | { ok: false; error: string };

/** Runs tasks so only the newest one reports: older results resolve to null. */
export function createLatestRunner() {
  let latest = 0;
  return {
    async run<T>(task: () => Promise<T>): Promise<Outcome<T> | null> {
      const id = ++latest;
      try {
        const value = await task();
        return id === latest ? { ok: true, value } : null;
      } catch (reason) { return id === latest ? { ok: false, error: errorMessage(reason) } : null; }
    },
    cancel() { latest += 1; },
  };
}

export const prepareArtwork = (invoke: Invoke, request: PrepareRequest) => invoke<PrepareResult>('prepare_artwork', { request });
export const saveArtwork = (invoke: Invoke, request: SaveRequest) => invoke<SaveResult>('save_artwork', { request });

export const offlineNote = 'This page prepares and previews artwork. To show an image on a key, assign it in the key artwork editor and start a run.';
