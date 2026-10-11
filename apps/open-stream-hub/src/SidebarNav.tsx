import type { ReactNode } from 'react';
import { CircleHelp, Grid2X2, Info, Gamepad2, Lightbulb, Link, PanelLeftClose, PanelLeftOpen, Settings } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { Page } from './ui-model';
import { useT } from './i18n-react';
import type { MessageKey } from './locales/en';

export const navStorageKey = 'open-stream-hub.sidebar';

// The sidebar is expanded unless the user collapsed it; storage may be missing or refuse access.
export function readNavCollapsed(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(navStorageKey) === 'collapsed'; } catch { return false; }
}

export function writeNavCollapsed(storage: Pick<Storage, 'setItem'> | null, collapsed: boolean): void {
  try { storage?.setItem(navStorageKey, collapsed ? 'collapsed' : 'expanded'); } catch { /* the sidebar stays as it is for this session */ }
}

const pages: { page: Page; label: MessageKey; icon: LucideIcon; hint?: MessageKey }[] = [
  { page: 'workspace', label: 'nav.workspace', icon: Grid2X2 },
  { page: 'lighting', label: 'nav.lighting', icon: Lightbulb, hint: 'nav.hint.lighting' },
  { page: 'demos', label: 'nav.demos', icon: Gamepad2, hint: 'nav.hint.demos' },
  { page: 'compatibility', label: 'nav.compatibility', icon: Link },
];

// The primary navigation. Collapsed, it shrinks to a column of icons whose names appear as tooltips
// beside them (the label stays as each button's accessible name), and the appearance picker moves to
// Settings, which has its own copy.
export function SidebarNav({ page, setPage, onGuide, themePicker, collapsed, onToggle }: {
  page: Page; setPage(page: Page): void; onGuide(): void; themePicker: ReactNode; collapsed: boolean; onToggle(): void;
}) {
  const t = useT();
  // Expanded buttons show their label; collapsed ones carry it as the tooltip instead.
  const tip = (label: string) => collapsed ? { 'data-tip': label, 'aria-label': label } : {};
  const current = (target: Page) => ({ 'aria-current': page === target ? ('page' as const) : undefined });
  const labelOf = (text: string) => collapsed ? null : <span className="nav-label">{text}</span>;
  return <aside className={`sidebar${collapsed ? ' collapsed' : ''}`}>
    <button type="button" className="nav-toggle" aria-expanded={!collapsed} aria-label={collapsed ? t('nav.expandTip') : t('nav.collapseTip')}
        data-tip={collapsed ? t('nav.expandTip') : undefined} onClick={onToggle}>
        {collapsed ? <PanelLeftOpen size={20} aria-hidden="true" /> : <><PanelLeftClose size={20} aria-hidden="true" /><span className="nav-label">{t('nav.collapse')}</span></>}
      </button>
    <div className="sidebar-scroll">
    <nav aria-label={t('nav.primary')}>
      {pages.map(({ page: target, label, icon: Icon, hint }) => <button key={target} className={page === target ? 'active' : ''} {...current(target)}
        {...tip(t(label))} title={collapsed || !hint ? undefined : t(hint)} onClick={() => setPage(target)}><Icon size={22} aria-hidden="true" />{labelOf(t(label))}</button>)}
    </nav>
    <div className="sidebar-bottom">
      <div className="sidebar-help">
        <button className="help-link" {...tip(t('nav.guide'))} onClick={onGuide}><CircleHelp size={22} aria-hidden="true" />{labelOf(t('nav.guide'))}</button>
        <button className={`help-link ${page === 'about' ? 'active' : ''}`} {...current('about')} {...tip(t('nav.about'))} onClick={() => setPage('about')}><Info size={22} aria-hidden="true" />{labelOf(t('nav.about'))}</button>
      </div>
      <button className={`settings-link ${page === 'settings' ? 'active' : ''}`} {...current('settings')} {...tip(t('nav.settings'))} onClick={() => setPage('settings')}><Settings size={22} aria-hidden="true" />{labelOf(t('nav.settings'))}</button>
      {!collapsed && themePicker}
    </div>
    </div>
  </aside>;
}
