import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { isValidElement } from 'react';
import type { ReactNode } from 'react';
import type { Profile } from './contracts';
import { starters } from './starter-model';
import { StarterDialog } from './StarterDialog';
import type { StarterEntry } from './StarterDialog';

// Follows the SSR test style: render as a function, walk the element tree,
// invoke the real handlers.
type AnyElement = { props: { children?: ReactNode; className?: string; labelledBy?: string; onClick?(): void; onCancel?(): void; 'data-starter'?: string } };
function elementsOf(node: ReactNode): AnyElement[] {
  if (node === null || node === undefined || typeof node === 'boolean' || typeof node === 'string' || typeof node === 'number') return [];
  if (Array.isArray(node)) return node.flatMap(elementsOf);
  if (isValidElement(node)) return [node as unknown as AnyElement, ...elementsOf((node.props as { children?: ReactNode }).children)];
  return [];
}

type DialogModule = Parameters<typeof StarterDialog>[0]['module'];
const pick: { profile: Profile | null } = { profile: null };
const closed = { count: 0, guide: 0 };
const resetPicks = () => { pick.profile = null; closed.count = 0; closed.guide = 0; };
const props = (entry: StarterEntry, module: DialogModule) => ({
  open: true, entry, module,
  onPick: (profile: Profile) => { pick.profile = profile; }, onClose: () => { closed.count += 1; }, onGuide: () => { closed.guide += 1; },
});
const render = (entry: StarterEntry, module: DialogModule) => renderToStaticMarkup(<StarterDialog {...props(entry, module)} />);
// StarterDialog is hook-free, so tests call it as a function and walk the
// returned element tree (the ModalDialog wrapper stays an unexecuted element).
const mounted = (entry: StarterEntry, module: DialogModule) => elementsOf(StarterDialog(props(entry, module)));

describe('starter chooser', () => {
  it('offers every starter on the first run, with the empty start and the guide', () => {
    const markup = render('first-run', null);
    expect(markup).toContain('Set up your first profile');
    for (const item of starters) {
      expect(markup).toContain(item.name);
      expect(markup).toContain(`data-starter="${item.id}"`);
      expect(markup).toContain(item.description);
    }
    expect(markup).toContain('Start with an empty profile');
    expect(markup).toContain('Getting started guide');
    expect(markup).toContain('executing actions always asks for confirmation');
  });
  it('titles the manager entry around the open profile', () => {
    const markup = render('manager', null);
    expect(markup).toContain('New from a starter');
    expect(markup).toContain('>Cancel<');
    expect(markup).not.toContain('Start with an empty profile');
  });
  it('offers only starters that use the connected module, and closes without picking', () => {
    const markup = render('module', 'm032le');
    expect(markup).toContain('2-roller scroll module connected');
    expect(markup).toContain('Mac basics');
    expect(markup).not.toContain('Streaming (OBS)');
    expect(markup).toContain('Not now');
    expect(markup).not.toContain('Getting started guide');

    resetPicks();
    const elements = mounted('module', 'm032le');
    elements.find(element => element.props['data-starter'] === 'streaming')?.props.onClick?.();
    expect(pick.profile).toBeNull(); // not offered for rollers
    elements.find(element => element.props['data-starter'] === 'mac-basics')?.props.onClick?.();
    expect(pick.profile?.name).toBe('Mac basics');
    elements.find(element => element.props.labelledBy === 'starter-title')?.props.onCancel?.();
    expect(closed.count).toBe(1);
  });
  it('picks a starter from the first run and opens the guide from it', () => {
    resetPicks();
    const elements = mounted('first-run', null);
    elements.find(element => element.props['data-starter'] === 'streaming')?.props.onClick?.();
    expect(pick.profile?.name).toBe('Streaming (OBS)');
    expect(pick.profile?.keypad?.pages[0].slots.filter(Boolean)).toHaveLength(5);
    // The ModalDialog element carries the cancel handler (its composed class
    // is built inside the wrapper).
    elements.find(element => element.props.labelledBy === 'starter-title')?.props.onCancel?.();
    expect(closed.count).toBe(1);
    resetPicks();
    elements.find(element => element.props.className?.includes('starter-guide'))?.props.onClick?.();
    expect(closed.guide).toBe(1);
  });
});
