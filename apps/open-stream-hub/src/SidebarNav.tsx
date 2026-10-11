import type { ReactNode } from 'react';
import { CircleHelp, Grid2X2, Info, Lightbulb, Link, Palette, PanelLeftClose, PanelLeftOpen, Settings } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { Page } from './ui-model';

export const navStorageKey = 'open-stream-hub.sidebar';

// The sidebar is expanded unless the user collapsed it; storage may be missing or refuse access.
export function readNavCollapsed(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(navStorageKey) === 'collapsed'; } catch { return false; }
}

export function writeNavCollapsed(storage: Pick<Storage, 'setItem'> | null, collapsed: boolean): void {
  try { storage?.setItem(navStorageKey, collapsed ? 'collapsed' : 'expanded'); } catch { /* the sidebar stays as it is for this session */ }
}

const pages: { page: Page; label: string; icon: LucideIcon; hint?: string }[] = [
  { page: 'workspace', label: 'Workspace', icon: Grid2X2 },
  { page: 'artwork', label: 'Display artwork', icon: Palette, hint: 'Prepare images for module displays offline' },
  { page: 'lighting', label: 'Lighting', icon: Lightbulb, hint: 'Dial backlight and colour, and knob, fader and base lighting' },
  { page: 'compatibility', label: 'Compatibility', icon: Link },
];

// The primary navigation. Collapsed, it shrinks to a column of icons whose names appear as tooltips
// beside them (the label stays as each button's accessible name), and the appearance picker moves to
// Settings, which has its own copy.
export function SidebarNav({ page, setPage, onGuide, themePicker, collapsed, onToggle }: {
  page: Page; setPage(page: Page): void; onGuide(): void; themePicker: ReactNode; collapsed: boolean; onToggle(): void;
}) {
  // Expanded buttons show their label; collapsed ones carry it as the tooltip instead.
  const tip = (label: string) => collapsed ? { 'data-tip': label, 'aria-label': label } : {};
  const current = (target: Page) => ({ 'aria-current': page === target ? ('page' as const) : undefined });
  const labelOf = (text: string) => collapsed ? null : <span className="nav-label">{text}</span>;
  return <aside className={`sidebar${collapsed ? ' collapsed' : ''}`}>
    <nav aria-label="Primary navigation">
      {pages.map(({ page: target, label, icon: Icon, hint }) => <button key={target} className={page === target ? 'active' : ''} {...current(target)}
        {...tip(label)} title={collapsed ? undefined : hint} onClick={() => setPage(target)}><Icon size={22} aria-hidden="true" />{labelOf(label)}</button>)}
    </nav>
    <div className="sidebar-bottom">
      <button className="help-link" {...tip('Getting started')} onClick={onGuide}><CircleHelp size={22} aria-hidden="true" />{labelOf('Getting started')}</button>
      <button className={`help-link ${page === 'about' ? 'active' : ''}`} {...current('about')} {...tip('About')} onClick={() => setPage('about')}><Info size={22} aria-hidden="true" />{labelOf('About')}</button>
      <button className={`settings-link ${page === 'settings' ? 'active' : ''}`} {...current('settings')} {...tip('Settings')} onClick={() => setPage('settings')}><Settings size={22} aria-hidden="true" />{labelOf('Settings')}</button>
      {!collapsed && themePicker}
      <button type="button" className="nav-toggle" aria-expanded={!collapsed} aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        data-tip={collapsed ? 'Expand sidebar' : 'Collapse sidebar'} onClick={onToggle}>
        {collapsed ? <PanelLeftOpen size={20} aria-hidden="true" /> : <><PanelLeftClose size={20} aria-hidden="true" /><span className="nav-label">Collapse</span></>}
      </button>
    </div>
  </aside>;
}
