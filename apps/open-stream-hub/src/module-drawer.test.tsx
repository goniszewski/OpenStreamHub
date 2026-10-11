import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ModuleDrawer } from './ModuleDrawer';
import { drawerStorageKey, drawerTiles, readDrawerOpen, reconcilePicks, writeDrawerOpen } from './module-drawer-model';
import type { DrawerTile } from './module-drawer-model';
import type { LightingDevice } from './lighting-model';
import { FakeStorage } from './fake-storage';

const base: LightingDevice = { path_hex: 'bs', module: 'm0064', serial_number: 'MHBS01AA461251601980' };
const dial: LightingDevice = { path_hex: 'dl', module: 'm131e', serial_number: 'MHDD01AA221252300843' };
const knob1: LightingDevice = { path_hex: 'k1', module: 'm003k', serial_number: 'MHKB03AA221252301179' };
const knob2: LightingDevice = { path_hex: 'k2', module: 'm003k', serial_number: 'MHKB03AA221252309999' };
const faders: LightingDevice = { path_hex: 'fd', module: 'm005s', serial_number: 'MHFD05AA421251601744' };
const keypad: LightingDevice = { path_hex: 'ke', module: 'm4315', serial_number: null };
const input = (over: Partial<Parameters<typeof drawerTiles>[0]> = {}) => ({
  page: 'lighting' as const, devices: [base, dial, knob1, faders, keypad], selectedPath: null, markedPaths: [] as string[], editorKind: 'm003k' as const, ...over,
});
const keys = (tiles: DrawerTile[], field: 'picked' | 'marked' | 'dimmed') => tiles.filter(tile => tile[field]).map(tile => tile.key);

describe('drawerTiles', () => {
  it('Lighting lists the connected units, picks the shared pick or the first dial, and dims what it cannot light', () => {
    const tiles = drawerTiles(input());
    expect(tiles.map(tile => tile.key)).toEqual(['bs', 'dl', 'k1', 'fd', 'ke']);
    expect(keys(tiles, 'picked')).toEqual(['dl']);
    expect(keys(tiles, 'dimmed')).toEqual(['ke']);
    expect(tiles[0].caption).toBe('Colour, effect and brightness · 1 LED');
    expect(keys(drawerTiles(input({ selectedPath: 'fd', markedPaths: ['k1', 'fd'] })), 'marked')).toEqual(['k1', 'fd']);
    expect(keys(drawerTiles(input({ selectedPath: 'fd' })), 'picked')).toEqual(['fd']);
  });
  it('Workspace lists every module kind once per unit, "Not connected" for the rest, and never the base', () => {
    const tiles = drawerTiles(input({ page: 'workspace', devices: [base, dial, knob1, knob2] }));
    expect(tiles.map(tile => tile.key)).toEqual(['dl', 'm4315', 'k1', 'k2', 'm005s', 'm032le']);
    expect(tiles.filter(tile => tile.path === null).map(tile => tile.caption)).toEqual(['Not connected', 'Not connected', 'Not connected']);
    // The editor's kind decides the pick; with two knob units the first is picked until the shared pick names one.
    expect(keys(tiles, 'picked')).toEqual(['k1']);
    expect(keys(drawerTiles(input({ page: 'workspace', devices: [dial, knob1, knob2], selectedPath: 'k2' })), 'picked')).toEqual(['k2']);
    // A shared pick of another kind does not override the editor's kind.
    expect(keys(drawerTiles(input({ page: 'workspace', devices: [dial, knob1], selectedPath: 'dl' })), 'picked')).toEqual(['k1']);
    // A kind that is not connected can still be the editor's pick.
    expect(keys(drawerTiles(input({ page: 'workspace', devices: [dial], editorKind: 'm005s' })), 'picked')).toEqual(['m005s']);
  });
});

describe('drawer open state', () => {
  it('is open until closed, and survives storage that refuses', () => {
    const storage = new FakeStorage();
    expect(readDrawerOpen(storage)).toBe(true);
    writeDrawerOpen(storage, false);
    expect(storage.getItem(drawerStorageKey)).toBe('closed');
    expect(readDrawerOpen(storage)).toBe(false);
    writeDrawerOpen(storage, true);
    expect(readDrawerOpen(storage)).toBe(true);
    const refusing = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    expect(readDrawerOpen(refusing)).toBe(true);
    expect(() => writeDrawerOpen(refusing, false)).not.toThrow();
    expect(readDrawerOpen(null)).toBe(true);
  });
});

describe('ModuleDrawer', () => {
  const tiles = drawerTiles(input({ selectedPath: 'fd', markedPaths: ['bs', 'fd'] }));

  it('shows a tile per module with photo, name, serial tail and caption, flagging the pick and marked ones', () => {
    const markup = renderToStaticMarkup(<ModuleDrawer tiles={tiles} open onToggle={() => {}} onPick={() => {}} />);
    expect(markup).toContain('aria-expanded="true"');
    expect(markup).toContain('5 connected');
    expect(markup).toContain('src="/modules/base-topdown.svg"');
    expect(markup).toContain('src="data:image/svg+xml,');
    expect(markup).toContain('<span class="module-tile-serial">…1744</span>');
    expect(markup).toMatch(/aria-label="5-fader precision module · …1744" class="module-tile picked marked"/);
    expect(markup).toMatch(/aria-label="MasterHUB base · …1980" class="module-tile marked"/);
    expect(markup.match(/Included in Whole kit/g)).toHaveLength(2);
    expect(markup).toMatch(/class="module-tile unlit"/);
  });
  it('closed, shrinks to one line naming the pick', () => {
    const markup = renderToStaticMarkup(<ModuleDrawer tiles={tiles} open={false} onToggle={() => {}} onPick={() => {}} />);
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).toContain('5-fader precision module · …1744');
    expect(markup).not.toContain('module-tile ');
  });
  it('says so when nothing is connected', () => {
    expect(renderToStaticMarkup(<ModuleDrawer tiles={[]} open onToggle={() => {}} onPick={() => {}} />)).toContain('No modules are connected.');
  });
});

describe('reconcilePicks', () => {
  const devices = [base, dial, knob1, knob2, faders];
  const settled = { path: 'dl', kind: 'm131e' as const };

  it('does nothing while the two picks agree', () => {
    expect(reconcilePicks({ previous: settled, pickPath: 'dl', editorKind: 'm131e', devices })).toEqual({ act: 'none', next: settled });
  });
  it('opens the kind of a unit picked elsewhere in the editor', () => {
    expect(reconcilePicks({ previous: settled, pickPath: 'k2', editorKind: 'm131e', devices }))
      .toEqual({ act: 'editor', kind: 'm003k', next: { path: 'k2', kind: 'm003k' } });
  });
  it('leaves the editor alone when the pick is the base, which has no bindings', () => {
    expect(reconcilePicks({ previous: settled, pickPath: 'bs', editorKind: 'm131e', devices }).act).toBe('none');
  });
  it('moves the shared pick to a unit of the kind opened in the editor', () => {
    expect(reconcilePicks({ previous: settled, pickPath: 'dl', editorKind: 'm005s', devices }))
      .toEqual({ act: 'selection', path: 'fd', next: { path: 'fd', kind: 'm005s' } });
  });
  it('keeps a pick that already is a unit of the editor kind, and ignores a kind with no unit', () => {
    expect(reconcilePicks({ previous: settled, pickPath: 'k2', editorKind: 'm003k', devices: [dial, knob1, knob2] }).act).toBe('none');
    expect(reconcilePicks({ previous: settled, pickPath: 'dl', editorKind: 'm032le', devices }).act).toBe('none');
  });
});
