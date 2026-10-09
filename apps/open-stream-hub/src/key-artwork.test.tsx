import { describe, expect, it } from 'vitest';
import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { BindingAction, KeyArtwork, Profile } from './contracts';
import { KeyArtworkPreview, KeyArtworkEditor, KeyDesignStudio } from './KeyArtworkEditor';
import { appIconId, defaultKeyArtworkDesign, initialKeyArtworkDesign } from './key-artwork-design';
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

// Direct element-tree helpers: KeyDesignStudio is hook-free, so tests call it as a
// function and invoke the handlers exactly as the browser would.
type AnyElement = ReactElement<{ [key: string]: unknown }>;

function elementsOf(node: ReactNode): AnyElement[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(elementsOf);
  if (isValidElement(node)) return [node as AnyElement, ...elementsOf((node.props as { children?: ReactNode }).children)];
  return [];
}
type StudioProps = Parameters<typeof KeyDesignStudio>[0];
const treeOf = (props: StudioProps) => KeyDesignStudio(props) as AnyElement;

const studioBase = (over: Partial<StudioProps> = {}): StudioProps => ({
  design: { ...defaultKeyArtworkDesign }, onDesign: () => {}, action: null, appIcon: null, appIconFailed: false,
  recents: [], query: '', onQuery: () => {}, libraryOpen: false, onLibraryOpen: () => {}, preview: null,
  disabled: false, busy: false, prepareAvailable: true, onApply: () => {}, ...over,
});

const buttonByLabel = (tree: AnyElement, label: string, group?: string) => {
  const scope = group ? elementsOf(tree).find(element => element.props['aria-label'] === group) : tree;
  const found = elementsOf(scope).filter(element => element.type === 'button' && element.props['aria-label'] === label);
  expect(found).toHaveLength(1);
  return found[0];
};

describe('key design studio', () => {
  it('opens with the key label as text and the first catalog suggestion preselected', () => {
    const mute = initialKeyArtworkDesign({ type: 'toggle_mute' }, 'Quiet');
    expect(mute.text).toBe('Quiet');
    expect(mute.iconId).toBe('volume-x');
    // The .app icon needs a lookup first, so the next catalog icon is picked.
    const launch = initialKeyArtworkDesign({ type: 'launch_app', bundle_id: 'com.apple.finder' });
    expect(launch.iconId).toBe('app-window');
    expect(initialKeyArtworkDesign(null).iconId).toBeNull();
  });

  it('suggests icons for the bound action and toggles the choice', () => {
    const designs: Parameters<StudioProps['onDesign']>[0][] = [];
    const tree = treeOf(studioBase({ action: { type: 'toggle_mute' }, onDesign: design => designs.push(design) }));
    const markup = renderToStaticMarkup(tree);
    expect(markup).toContain('Suggested');
    expect(markup).toContain('aria-label="Muted"');
    expect(markup).toContain('aria-label="Microphone off"');
    (buttonByLabel(tree, 'Muted', 'Suggested icons').props as { onClick(): void }).onClick();
    expect(designs).toHaveLength(1);
    expect(designs[0].iconId).toBe('volume-x');
    // Pressing the selected icon again clears it.
    const selected = treeOf(studioBase({ action: { type: 'toggle_mute' }, design: { ...defaultKeyArtworkDesign, iconId: 'volume-x' }, onDesign: design => designs.push(design) }));
    (buttonByLabel(selected, 'Muted', 'Suggested icons').props as { onClick(): void }).onClick();
    expect(designs[1].iconId).toBeNull();
  });

  it('carries the search query, recents and library open state', () => {
    const queries: string[] = [];
    const opens: boolean[] = [];
    const tree = treeOf(studioBase({ recents: ['globe', 'play'], onQuery: query => queries.push(query), onLibraryOpen: open => opens.push(open) }));
    const markup = renderToStaticMarkup(tree);
    expect(markup).toContain('Recently used');
    expect(markup).toContain('aria-label="Website"');
    expect(markup).toContain('All icons');
    const search = elementsOf(tree).find(element => element.props.id === 'key-icon-search');
    (search!.props as { onChange(event: { target: { value: string } }): void }).onChange({ target: { value: 'loud' } });
    expect(queries).toEqual(['loud']);
    (elementsOf(tree).find(element => element.type === 'details')!.props as { onToggle(event: { target: { open: boolean } }): void }).onToggle({ target: { open: true } });
    expect(opens).toEqual([true]);
    expect(renderToStaticMarkup(treeOf(studioBase({ query: 'nothing matches' })))).toContain('No icon matches.');
  });

  it('edits text, size, alignment and colours through the controlled handlers', () => {
    const designs: Parameters<StudioProps['onDesign']>[0][] = [];
    const tree = treeOf(studioBase({ onDesign: design => designs.push(design) }));
    (elementsOf(tree).find(element => element.props.id === 'key-design-text')!.props as { onChange(event: { target: { value: string } }): void }).onChange({ target: { value: 'Mute' } });
    expect(designs[0].text).toBe('Mute');
    const sizeButtons = elementsOf(tree).filter(element => element.type === 'button' && (element.props as { 'aria-pressed'?: boolean })['aria-pressed'] !== undefined);
    expect(sizeButtons.length).toBeGreaterThanOrEqual(7);
    const large = sizeButtons.find(element => element.props.children === 'Large');
    (large!.props as { onClick(): void }).onClick();
    expect(designs[1].textSize).toBe(2);
    const colour = elementsOf(tree).find(element => element.props.id === 'key-design-background');
    (colour!.props as { onChange(event: { target: { value: string } }): void }).onChange({ target: { value: '#102030' } });
    expect(designs[2].background).toBe('#102030');
  });

  it('gates applying on a valid design and the desktop pipeline', () => {
    const applies: number[] = [];
    const ready = treeOf(studioBase({ design: { ...defaultKeyArtworkDesign, iconId: 'volume-2', text: 'Mute' }, onApply: () => applies.push(1) }));
    const applyButton = elementsOf(ready).find(element => element.type === 'button' && (element.props.children as string[] ?? []).includes?.('Set key artwork'));
    (applyButton!.props as { onClick(): void }).onClick();
    expect(applies).toHaveLength(1);
    // An invalid colour is shown and blocks applying.
    const invalid = treeOf(studioBase({ design: { ...defaultKeyArtworkDesign, background: 'zzz' } }));
    expect(renderToStaticMarkup(invalid)).toContain('Background colour must be a hex value');
    expect(elementsOf(invalid).find(element => (element.props.children as string)?.includes?.('Set key artwork'))?.props.disabled).toBe(true);
    // Without the desktop app there is nothing to prepare onto.
    expect(elementsOf(treeOf(studioBase({ prepareAvailable: false }))).find(element => (element.props.children as string)?.includes?.('Set key artwork'))?.props.disabled).toBe(true);
  });

  it('offers the fetched app icon and blocks applying until it has loaded', () => {
    const action: BindingAction = { type: 'launch_app', bundle_id: 'com.apple.finder' };
    const loading = renderToStaticMarkup(treeOf(studioBase({ action })));
    expect(loading).toContain('aria-label="Application icon"');
    expect(loading).toContain('key-icon-loading');
    expect(buttonByLabel(treeOf(studioBase({ action })), 'Application icon').props.disabled).toBe(true);
    const designs: Parameters<StudioProps['onDesign']>[0][] = [];
    const loaded = treeOf(studioBase({ action, appIcon: { bundleId: 'com.apple.finder', pngBase64: 'QUJD' }, onDesign: design => designs.push(design) }));
    expect(renderToStaticMarkup(loaded)).toContain('data:image/png;base64,QUJD');
    (buttonByLabel(loaded, 'Application icon', 'Suggested icons').props as { onClick(): void }).onClick();
    expect(designs[0].iconId).toBe(appIconId('com.apple.finder'));
    // A failed lookup removes the chip; the catalog fallback stays.
    expect(renderToStaticMarkup(treeOf(studioBase({ action, appIconFailed: true })))).not.toContain('Application icon');
    // A chosen app icon cannot be applied without its image.
    const chosen = treeOf(studioBase({ action, design: { ...defaultKeyArtworkDesign, iconId: appIconId('com.apple.finder') } }));
    expect(elementsOf(chosen).find(element => (element.props.children as string)?.includes?.('Set key artwork'))?.props.disabled).toBe(true);
  });
});
