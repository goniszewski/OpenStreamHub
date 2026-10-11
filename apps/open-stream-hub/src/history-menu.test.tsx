import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { FakeStorage } from './fake-storage';
import { HistoryList, HistoryMenu, historyTipStorageKey, markHistoryTipShown, readHistoryTipShown } from './HistoryMenu';

const hints = { undoHint: '⌘Z', redoHint: '⌘⇧Z' };

describe('HistoryList', () => {
  it('lists the changes to redo above the changes to undo, each as a button, and reports how many steps a pick spans', () => {
    const picks: string[] = [];
    const tree = HistoryList({ undo: ['Key 2: Mute', 'Renamed the profile to “B”'], redo: ['Changed the dial display'], showTip: false, ...hints,
      onUndo: steps => picks.push(`undo ${steps}`), onRedo: steps => picks.push(`redo ${steps}`) });
    const markup = renderToStaticMarkup(tree);
    expect(markup.indexOf('Redo')).toBeLessThan(markup.indexOf('Undo'));
    for (const label of ['Key 2: Mute', 'Renamed the profile to “B”', 'Changed the dial display']) expect(markup).toContain(`>${label}</button>`);
    expect(markup).toContain('aria-label="Changes you can undo, latest first"');
    expect(markup).not.toContain('role="note"');
    // The list is plain elements, so the picks can be driven through their props.
    const buttons: { props: { onClick(): void; children: string } }[] = [];
    const walk = (node: unknown): void => {
      if (Array.isArray(node)) { node.forEach(walk); return; }
      if (!node || typeof node !== 'object' || !('props' in node)) return;
      const element = node as { type: unknown; props: { children?: unknown; onClick?(): void } };
      if (element.type === 'button') buttons.push(element as never);
      walk(element.props.children);
    };
    walk(tree);
    buttons.forEach(button => button.props.onClick());
    expect(picks).toEqual(['redo 1', 'undo 1', 'undo 2']);
  });

  it('says so when there is nothing to undo, and shows the one-time tip with the shortcuts', () => {
    const empty = renderToStaticMarkup(HistoryList({ undo: [], redo: [], showTip: true, ...hints, onUndo: () => {}, onRedo: () => {} }));
    expect(empty).toContain('Nothing to undo yet.');
    expect(empty).toContain('role="note"');
    expect(empty).toContain('⌘Z undoes the last change and ⌘⇧Z repeats it.');
    expect(empty).not.toContain('Redo');
  });
});

describe('HistoryMenu', () => {
  const render = (props: Partial<Parameters<typeof HistoryMenu>[0]> = {}) => renderToStaticMarkup(
    <HistoryMenu undo={['A']} redo={[]} disabled={false} onUndo={() => {}} onRedo={() => {}} storage={new FakeStorage()} {...hints} {...props} />);

  it('is a closed button with a tooltip that says what it does, and stays closed until clicked', () => {
    const markup = render();
    expect(markup).toContain('aria-label="Change history"');
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).toContain('data-tip="Change history: see and jump back through recent changes"');
    expect(markup).not.toContain('role="dialog"');
    expect(render({ undo: [] })).toContain('data-tip="Change history (nothing to undo yet)"');
    expect(render({ disabled: true })).toContain('disabled=""');
  });
});

describe('the one-time tip', () => {
  it('shows until the list has been opened once, and survives storage that refuses', () => {
    const storage = new FakeStorage();
    expect(readHistoryTipShown(storage)).toBe(false);
    markHistoryTipShown(storage);
    expect(storage.getItem(historyTipStorageKey)).toBe('shown');
    expect(readHistoryTipShown(storage)).toBe(true);
    const refusing = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    expect(readHistoryTipShown(refusing)).toBe(false);
    expect(() => markHistoryTipShown(refusing)).not.toThrow();
    expect(readHistoryTipShown(null)).toBe(false);
  });
});
