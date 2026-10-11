import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { SidebarNav, navStorageKey, readNavCollapsed, writeNavCollapsed } from './SidebarNav';
import { FakeStorage } from './fake-storage';
import type { Page } from './ui-model';

const render = (collapsed: boolean, page: Page = 'lighting') => renderToStaticMarkup(
  <SidebarNav page={page} setPage={() => {}} onGuide={() => {}} themePicker={<div className="theme-picker">System</div>} collapsed={collapsed} onToggle={() => {}} />);

describe('SidebarNav', () => {
  it('expanded, shows every label, the appearance picker and a Collapse button, with no tooltips', () => {
    const markup = render(false);
    for (const label of ['Workspace', 'Display artwork', 'Lighting', 'Compatibility', 'Getting started', 'About', 'Settings']) {
      expect(markup).toContain(`<span class="nav-label">${label}</span>`);
    }
    expect(markup).toContain('class="theme-picker"');
    expect(markup).toContain('aria-label="Collapse sidebar"');
    expect(markup).toContain('aria-expanded="true"');
    expect(markup).not.toContain('class="sidebar collapsed"');
    expect(markup).not.toContain('data-tip="Workspace"');
    expect(markup).toContain('title="Prepare images for module displays offline"');
  });
  it('collapsed, keeps each page reachable as an icon named by its tooltip and accessible label', () => {
    const markup = render(true);
    expect(markup).toContain('class="sidebar collapsed"');
    expect(markup).not.toContain('nav-label');
    expect(markup).not.toContain('class="theme-picker"');
    for (const label of ['Workspace', 'Display artwork', 'Lighting', 'Compatibility', 'Getting started', 'About', 'Settings']) {
      expect(markup).toContain(`data-tip="${label}" aria-label="${label}"`);
    }
    expect(markup).toContain('aria-label="Expand sidebar" data-tip="Expand sidebar"');
    expect(markup).toContain('aria-expanded="false"');
  });
  it('marks the current page in both states', () => {
    for (const collapsed of [false, true]) {
      expect(render(collapsed, 'about').match(/aria-current="page"/g)).toHaveLength(1);
      expect(render(collapsed, 'about')).toMatch(/class="help-link active" aria-current="page"/);
    }
    expect(render(false, 'settings')).toMatch(/class="settings-link active" aria-current="page"/);
    expect(render(false, 'lighting')).toMatch(/class="active" aria-current="page"[^>]*>(<svg[^>]*>.*?<\/svg>)<span class="nav-label">Lighting/);
  });
});

describe('sidebar storage', () => {
  it('is expanded until collapsed, and survives storage that refuses', () => {
    const storage = new FakeStorage();
    expect(readNavCollapsed(storage)).toBe(false);
    writeNavCollapsed(storage, true);
    expect(storage.getItem(navStorageKey)).toBe('collapsed');
    expect(readNavCollapsed(storage)).toBe(true);
    writeNavCollapsed(storage, false);
    expect(readNavCollapsed(storage)).toBe(false);
    const refusing = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    expect(readNavCollapsed(refusing)).toBe(false);
    expect(() => writeNavCollapsed(refusing, true)).not.toThrow();
    expect(readNavCollapsed(null)).toBe(false);
  });
});
