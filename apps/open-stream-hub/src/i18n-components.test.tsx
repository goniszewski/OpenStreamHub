import { afterEach, describe, expect, it } from 'vitest';
import { SessionView } from './ProfileManager';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { i18n, translateIn } from './i18n-react';
import { SidebarNav } from './SidebarNav';
import { ModuleDrawer } from './ModuleDrawer';
import { HistoryList, HistoryMenu } from './HistoryMenu';
import { LanguagePanel } from './InfoPages';
import { FakeStorage } from './fake-storage';
import type { DrawerTile } from './module-drawer-model';

afterEach(() => i18n.setLanguage('en'));

const tile = (key: string, picked = false): DrawerTile => ({ key, module: 'm003k', path: key, serial: null, caption: 'caption', picked, marked: false, dimmed: false });
const nav = () => renderToStaticMarkup(<SidebarNav page="workspace" setPage={() => {}} onGuide={() => {}} themePicker={null} collapsed={false} onToggle={() => {}} />);
const drawer = (count: number, open = true) => renderToStaticMarkup(<ModuleDrawer tiles={Array.from({ length: count }, (_, index) => tile(`k${index}`, index === 0))} open={open} onToggle={() => {}} onPick={() => {}} />);

describe('components in Polish', () => {
  it('draw the sidebar in the chosen language and switch back without a restart', () => {
    expect(nav()).toContain('Workspace');
    i18n.setLanguage('pl');
    const polish = nav();
    for (const word of ['Obszar roboczy', 'Grafiki wyświetlaczy', 'Podświetlenie', 'Zgodność', 'Pierwsze kroki', 'O programie', 'Ustawienia', 'Zwiń']) expect(polish).toContain(word);
    expect(polish).toContain('aria-label="Nawigacja główna"');
    expect(polish).toContain('title="Przygotuj obrazy dla wyświetlaczy modułów bez połączenia"');
    expect(polish).not.toContain('Workspace');
    i18n.setLanguage('en');
    expect(nav()).toContain('Workspace');
  });
  it('draw the drawer with Polish plural forms for the connected count', () => {
    i18n.setLanguage('pl');
    expect(drawer(1)).toContain('1 podłączony');
    expect(drawer(2)).toContain('2 podłączone');
    expect(drawer(5)).toContain('5 podłączonych');
    expect(drawer(22)).toContain('22 podłączone');
    expect(drawer(1)).toContain('Moduły');
    expect(drawer(1, false)).toContain('Pokaż listę modułów');
    expect(renderToStaticMarkup(<ModuleDrawer tiles={[]} open onToggle={() => {}} onPick={() => {}} />)).toContain('Nie podłączono żadnych modułów.');
    i18n.setLanguage('en');
    expect(drawer(2)).toContain('2 connected');
  });
  it('draw the history menu and its list in Polish', () => {
    i18n.setLanguage('pl');
    const menu = renderToStaticMarkup(<HistoryMenu undo={['A']} redo={[]} disabled={false} onUndo={() => {}} onRedo={() => {}} storage={new FakeStorage()} undoHint="⌘Z" redoHint="⌘⇧Z" />);
    expect(menu).toContain('aria-label="Historia zmian"');
    expect(menu).toContain('data-tip="Historia zmian: zobacz ostatnie zmiany i cofnij się do wybranej"');
    const list = renderToStaticMarkup(HistoryList({ undo: ['A'], redo: ['B'], showTip: true, undoHint: '⌘Z', redoHint: '⌘⇧Z', onUndo: () => {}, onRedo: () => {}, t: translateIn('pl') }));
    expect(list).toContain('⌘Z cofa ostatnią zmianę, a ⌘⇧Z ją ponawia.');
    expect(list).toContain('>Ponów<');
    expect(list).toContain('>Cofnij<');
    expect(renderToStaticMarkup(HistoryList({ undo: [], redo: [], showTip: false, undoHint: '', redoHint: '', onUndo: () => {}, onRedo: () => {}, t: translateIn('pl') }))).toContain('Nie ma jeszcze czego cofać.');
  });
  it('offer the language picker in Settings with each language named in itself', () => {
    const markup = renderToStaticMarkup(<LanguagePanel />);
    expect(markup).toMatch(/<option value="system"[^>]*>System default<\/option>/);
    expect(markup).toMatch(/<option value="en"[^>]*>English<\/option>/);
    expect(markup).toMatch(/<option value="pl"[^>]*>Polski<\/option>/);
    // The current setting is the selected one.
    expect(markup).toMatch(/<option value="en" selected="">English<\/option>/);
    i18n.setLanguage('pl');
    expect(renderToStaticMarkup(<LanguagePanel />)).toContain('Domyślny systemowy');
  });
});

describe('profile manager parts', () => {
  it('draws the sample results in Polish when given the Polish translator', () => {
    const idle = { status: 'idle' } as never;
    expect(renderToStaticMarkup(<SessionView session={idle} t={translateIn('pl')} />)).toContain('Wyniki pojawią się tutaj');
    expect(renderToStaticMarkup(<SessionView session={idle} />)).toContain('Results appear here');
  });
});

// The files already moved to message keys must not grow new hard-coded text: English words in JSX text or in the
// attributes a person reads or hears (labels, tooltips, titles, placeholders) fail here. Files are added as they are
// converted; the rest of the app is still inline text.
const converted = ['SidebarNav.tsx', 'ModuleDrawer.tsx', 'HistoryMenu.tsx', 'ProfileManager.tsx', 'AutoSwitchRow.tsx', 'FocusRulesPanel.tsx'];
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
