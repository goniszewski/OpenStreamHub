import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { KeyArtwork, Profile } from './contracts';
import { KeyArtworkPreview, KeyArtworkEditor } from './KeyArtworkEditor';
import { keyArtworkFrame, setKeyArtwork } from './key-artwork';
import { clearKeypadSlot, createKeypadFolder, editKeypadBinding, emptyKeypadPage, renameKeypadAction } from './keypad-model';
import { KeyDisplayContent } from './DisplayContent';
import type { ProfileSession } from './profile-session';

const artwork: KeyArtwork = { frames: [{ jpeg_base64: 'AAAA', delay_ms: 100 }, { jpeg_base64: 'BBBB', delay_ms: 200 }], loop_count: 0 };
const profile = (): Profile => ({ schema_version: 2, name: 'Art', bindings: [], keypad: { pages: [emptyKeypadPage()] } });

describe('per-key artwork', () => {
  it('respects variable delays, skips late frames and holds the last finite frame', () => {
    expect([0, 99, 100, 299, 300, 400, 123400].map(time => keyArtworkFrame(artwork, time))).toEqual([0, 0, 1, 1, 0, 1, 1]);
    const finite = { ...artwork, loop_count: 2 };
    expect(keyArtworkFrame(finite, 300)).toBe(0);
    expect(keyArtworkFrame(finite, 599)).toBe(1);
    expect(keyArtworkFrame(finite, 600)).toBe(1);
    expect(keyArtworkFrame(finite, 999999)).toBe(1);
  });

  it('preserves artwork through binding edits, labels and folder creation', () => {
    let draft = setKeyArtwork(profile(), ['root'], 0, artwork);
    draft = editKeypadBinding(draft, ['root'], 0, 'pressed', '', { type: 'toggle_mute' }, 'Mute');
    draft = renameKeypadAction(draft, ['root'], 0, 'Quiet');
    draft = createKeypadFolder(draft, ['root'], 1, 'tools');
    expect(draft.keypad!.pages[0].artwork!['0']).toEqual(artwork);
    expect(JSON.parse(JSON.stringify(draft)).keypad.pages[0].artwork['0']).toEqual(artwork);
    expect(clearKeypadSlot(draft, ['root'], 0).keypad!.pages[0].artwork).toBeUndefined();
  });

  it('can attach to empty and folder keys, remove it, and protects generated Back', () => {
    let draft = createKeypadFolder(profile(), ['root'], 2, 'tools');
    draft = setKeyArtwork(draft, ['root'], 2, artwork);
    expect(draft.keypad!.pages[0].slots[2]?.type).toBe('folder');
    expect(setKeyArtwork(draft, ['root'], 2, null).keypad!.pages[0].artwork).toBeUndefined();
    expect(() => setKeyArtwork(draft, ['root', 'tools'], 0, artwork)).toThrow('Back');
    expect(() => setKeyArtwork(draft, ['root'], 15, artwork)).toThrow();
  });

  it('rejects profile byte overflow without changing the original draft', () => {
    const original = profile();
    expect(() => setKeyArtwork(original, ['root'], 0, { frames: [{ jpeg_base64: 'A'.repeat(1400000), delay_ms: 100 }], loop_count: 1 })).toThrow('limit');
    expect(original.keypad!.pages[0].artwork).toBeUndefined();
  });

  it('shows prepared artwork on empty keys and keeps Back visible', () => {
    const markup = renderToStaticMarkup(<KeyDisplayContent slot={null} pages={[]} depth={0} back={false} artwork={artwork} />);
    expect(markup).toContain('data:image/jpeg;base64,AAAA');
    expect(markup).not.toContain('display-empty');
    expect(renderToStaticMarkup(<KeyDisplayContent slot={null} pages={[]} depth={1} back artwork={artwork} />)).toContain('Back');
    expect(renderToStaticMarkup(<KeyArtworkPreview artwork={artwork} />)).toContain('alt="Key artwork"');
  });

  it('offers GIF and static file selection with limits and disables prep outside desktop', () => {
    const markup = renderToStaticMarkup(<KeyArtworkEditor session={{} as ProfileSession} path={['root']} control={0} artwork={artwork} disabled={false} />);
    expect(markup).toContain('image/gif');
    expect(markup).toContain('120 frames');
    expect(markup).toContain('Remove artwork');
    expect(markup).toMatch(/type="file"[^>]*disabled/);
    expect(markup).toContain('needs the desktop app');
  });
});
