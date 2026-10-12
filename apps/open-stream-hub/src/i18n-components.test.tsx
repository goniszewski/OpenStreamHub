import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { SidebarNav } from './SidebarNav';
import { ModuleDrawer } from './ModuleDrawer';
import { HistoryList } from './HistoryMenu';
import { RunHeaderControl, idleRunSession } from './ServicePanel';
import type { DrawerTile } from './module-drawer-model';

const tile = (key: string, picked = false): DrawerTile => ({ key, module: 'm003k', path: key, serial: null, caption: 'caption', picked, marked: false, dimmed: false });

describe('components draw their messages', () => {
  it('the sidebar', () => {
    const nav = renderToStaticMarkup(<SidebarNav page="workspace" setPage={() => {}} onGuide={() => {}} themePicker={null} collapsed={false} onToggle={() => {}} />);
    expect(nav).toContain('Workspace');
  });
  it('the drawer, with the count in the right plural form', () => {
    const drawer = (count: number) => renderToStaticMarkup(<ModuleDrawer tiles={Array.from({ length: count }, (_, index) => tile(`k${index}`, index === 0))} open onToggle={() => {}} onPick={() => {}} />);
    expect(drawer(1)).toContain('1 connected');
    expect(drawer(2)).toContain('2 connected');
  });
  it('the history list', () => {
    const list = renderToStaticMarkup(HistoryList({ undo: ['A'], redo: ['B'], showTip: true, undoHint: '⌘Z', redoHint: '⌘⇧Z', onUndo: () => {}, onRedo: () => {} }));
    expect(list).toContain('>Redo<');
    expect(list).toContain('>Undo<');
  });
  it('the run header', () => {
    expect(renderToStaticMarkup(<RunHeaderControl run={idleRunSession} />)).toContain('aria-label="Test mode"');
  });
});

// The files already moved to message keys must not grow new hard-coded text: English words in JSX text or in the
// attributes a person reads or hears (labels, tooltips, titles, placeholders) fail here. Files are added as they are
// converted; the rest of the app is still inline text.
const converted = ['SidebarNav.tsx', 'ModuleDrawer.tsx', 'HistoryMenu.tsx', 'ProfileManager.tsx', 'AutoSwitchRow.tsx', 'FocusRulesPanel.tsx', 'StarterDialog.tsx', 'ProfileTransfer.tsx', 'ServicePanel.tsx', 'Inspector.tsx'];
const jsxText = /(?<![=-])>\s*([A-Za-z][^<>{}`]*[A-Za-z.…])\s*</g;
const readableAttribute = /\b(?:aria-label|title|data-tip|placeholder|alt)="([^"{]*[A-Za-z]{2,}[^"]*)"/g;

describe('converted components', () => {
  for (const file of converted) {
    it(`${file} holds no hard-coded visible text`, () => {
      // Comments and type annotations are not rendered.
      const source = readFileSync(new URL(`./${file}`, import.meta.url), 'utf8').split('\n').filter(line => !line.trim().startsWith('//')).join('\n');
      const found = [...source.matchAll(jsxText)].map(match => match[1].trim()).filter(text => !/^[…·]/.test(text) && !/^(?:string|number|boolean|void)\b/.test(text));
      expect(found, file).toEqual([]);
      expect([...source.matchAll(readableAttribute)].map(match => match[1]), file).toEqual([]);
    });
  }
});
