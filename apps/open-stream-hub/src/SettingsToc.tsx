import { useEffect, useRef, useState } from 'react';

interface TocItem { id: string; text: string }

const slug = (text: string, index: number) => `settings-${index}-${text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')}`;

// A table of contents for the Settings page, built from the headings of its panels (several panels come from
// other components, so the list is read from the page itself). The run panel keeps its heading beside a status
// badge, so its heading sits one level deeper. It follows the scroll and jumps to a section.
export interface TocGroup { label: string; headings: string[] }

// `groups` name the headings that belong to each labelled group; every other heading goes under `defaultGroup`.
export function SettingsToc({ label, groups = [], defaultGroup }: { label: string; groups?: TocGroup[]; defaultGroup?: string }) {
  const nav = useRef<HTMLElement>(null);
  const [items, setItems] = useState<TocItem[]>([]);
  const [active, setActive] = useState<string | null>(null);

  useEffect(() => {
    const page = nav.current?.parentElement;
    if (!page) return;
    const read = () => {
      const headings = [...page.querySelectorAll<HTMLElement>(':scope > .panel > h2, :scope > .service-panel > .section-heading > h2')];
      const next = headings.map((heading, index) => {
        if (!heading.id) heading.id = slug(heading.textContent ?? '', index);
        heading.style.scrollMarginTop = '24px';
        return { id: heading.id, text: heading.textContent ?? '' };
      });
      setItems(previous => previous.length === next.length && previous.every((item, index) => item.id === next[index].id && item.text === next[index].text) ? previous : next);
    };
    read();
    const mutations = new MutationObserver(read);
    mutations.observe(page, { childList: true, subtree: true, characterData: true });
    return () => mutations.disconnect();
  }, []);

  useEffect(() => {
    if (items.length === 0 || typeof IntersectionObserver === 'undefined') return;
    const seen = new Set<string>();
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) { if (entry.isIntersecting) seen.add(entry.target.id); else seen.delete(entry.target.id); }
      const first = items.find(item => seen.has(item.id));
      if (first) setActive(first.id);
    }, { rootMargin: '0px 0px -60% 0px' });
    for (const item of items) { const element = document.getElementById(item.id); if (element) observer.observe(element); }
    return () => observer.disconnect();
  }, [items]);

  const go = (id: string) => {
    setActive(id);
    const target = document.getElementById(id);
    if (!target) return;
    const behavior = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
    // Scroll only the content pane: scrollIntoView also moves the clipped ancestors (the app shell and the
    // sidebar), which slides the header and navigation out of reach.
    const pane = target.closest<HTMLElement>('.main-content');
    if (!pane) { target.scrollIntoView({ behavior, block: 'start' }); return; }
    const top = target.getBoundingClientRect().top - pane.getBoundingClientRect().top + pane.scrollTop - 24;
    pane.scrollTo({ top: Math.max(0, top), behavior });
  };

  const link = (item: TocItem) => <li key={item.id}>
    <a href={`#${item.id}`} className={active === item.id ? 'active' : ''} aria-current={active === item.id ? 'location' : undefined}
      onClick={event => { event.preventDefault(); go(item.id); }}>{item.text}</a>
  </li>;
  const labelled = defaultGroup ? [{ label: defaultGroup, headings: [] as string[] }, ...groups] : [];
  const groupOf = (item: TocItem) => Math.max(0, labelled.findIndex((group, index) => index > 0 && group.headings.includes(item.text)));

  return <nav ref={nav} className="settings-toc" aria-label={label} hidden={items.length === 0}>
    {labelled.length === 0 ? <ul>{items.map(link)}</ul> : labelled.map((group, index) => {
      const members = items.filter(item => groupOf(item) === index);
      return members.length === 0 ? null : <div key={group.label} className="settings-toc-group"><p className="settings-toc-label">{group.label}</p><ul>{members.map(link)}</ul></div>;
    })}
  </nav>;
}
