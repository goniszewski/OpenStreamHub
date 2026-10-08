import type { KeyArtwork, Profile } from './contracts';
import { keypadIsBack, keypadPagesAtPath } from './keypad-model';

/** Pick the current frame from elapsed page time, skipping missed frames. */
export function keyArtworkFrame(artwork: KeyArtwork, elapsedMs: number): number {
  const duration = artwork.frames.reduce((total, frame) => total + frame.delay_ms, 0);
  if (!duration || artwork.frames.length < 2) return 0;
  const elapsed = Math.max(0, elapsedMs);
  if (artwork.loop_count > 0 && elapsed >= duration * artwork.loop_count) return artwork.frames.length - 1;
  let time = elapsed % duration;
  for (let index = 0; index < artwork.frames.length; index++) {
    if (time < artwork.frames[index].delay_ms) return index;
    time -= artwork.frames[index].delay_ms;
  }
  return artwork.frames.length - 1;
}

export function setKeyArtwork(profile: Profile, path: readonly string[], control: number, value: KeyArtwork | null): Profile {
  if (!Number.isInteger(control) || control < 0 || control >= 15 || keypadIsBack(path, control)) throw new Error('The Back key keeps its generated artwork.');
  const page = keypadPagesAtPath(profile, path).at(-1)!;
  const artwork = { ...page.artwork };
  if (value) artwork[String(control)] = value; else delete artwork[String(control)];
  const replacement = { ...page, artwork: Object.keys(artwork).length ? artwork : undefined };
  const updated = { ...profile, keypad: { ...profile.keypad!, pages: profile.keypad!.pages.map(item => item.id === page.id ? replacement : item) } };
  const totalBytes = updated.keypad.pages.reduce((total, item) => total + Object.values(item.artwork ?? {}).reduce((sum, asset) => sum + asset.frames.reduce((bytes, frame) => bytes + frame.jpeg_base64.length * 3 / 4 - (frame.jpeg_base64.endsWith('==') ? 2 : frame.jpeg_base64.endsWith('=') ? 1 : 0), 0), 0), 0);
  if (totalBytes > 700 * 1024) throw new Error('Artwork exceeds the profile’s 700 KiB prepared-image limit. Use a shorter or simpler GIF.');
  if (new TextEncoder().encode(JSON.stringify(updated, null, 2)).length > 1024 * 1024) throw new Error('This artwork would exceed the 1 MiB profile limit. Use a shorter or simpler GIF.');
  return updated;
}
