import { describe, expect, it } from 'vitest';
import type { Binding } from './contracts';
import { bundledPacks, loadPacks, packActionIssue, packIssues, packModules, parsePack } from './preset-packs';
import type { PresetPack } from './preset-packs';

// A small valid pack to break one field at a time.
const base = (): PresetPack => structuredClone(bundledPacks.packs.find(pack => pack.id === 'zoom')!);
const bind = (patch: Partial<Binding>): Binding => ({ module: 'm131e', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' }, ...patch });
const withProfile = (patch: (pack: PresetPack) => void) => { const pack = base(); patch(pack); return pack; };

describe('bundled packs', () => {
  it('are all valid, with no file refused', () => {
    expect(bundledPacks.rejected).toEqual([]);
    expect(bundledPacks.packs.map(pack => pack.id)).toEqual(['audacity', 'davinci-resolve', 'final-cut-pro', 'logic-pro', 'microsoft-teams', 'music-and-spotify', 'zoom']);
  });
  it('record a licence, a source and whether they were checked, and name the app they are for', () => {
    for (const pack of bundledPacks.packs) {
      expect(pack.license, pack.id).toBe('CC0-1.0');
      expect(pack.source.length, pack.id).toBeGreaterThan(30);
      expect(pack.verified, pack.id).toBe(false);
      expect(pack.app.name, pack.id).not.toBe('');
      expect(pack.focus_rule?.bundle_id, pack.id).toBe(pack.app.bundle_id);
    }
  });
  it('set up the modules they claim, and fill the 15-key page', () => {
    const resolve = bundledPacks.packs.find(pack => pack.id === 'davinci-resolve')!;
    expect([...packModules(resolve)].sort()).toEqual(['m003k', 'm005s', 'm131e', 'm4315']);
    for (const pack of bundledPacks.packs) expect(pack.profile.keypad!.pages[0].slots.some(slot => slot !== null), pack.id).toBe(true);
  });
});

describe('packIssues', () => {
  it('accepts a bundled pack and nothing is wrong with it', () => {
    expect(packIssues(base())).toEqual([]);
  });
  it('refuses what is not an object, an unknown field and a wrong version', () => {
    expect(packIssues(null)).toEqual(['A pack is a JSON object.']);
    expect(packIssues([])).toEqual(['A pack is a JSON object.']);
    expect(packIssues({ ...base(), colour: 'red' })).toContain('Unknown field "colour".');
    expect(packIssues({ ...base(), pack_version: 2 })).toContain('pack_version must be 1.');
  });
  it('checks the id, the texts, the licence, the source and the verified flag', () => {
    expect(packIssues({ ...base(), id: 'Bad Id' }).join()).toContain('id must be');
    expect(packIssues({ ...base(), name: '  ' }).join()).toContain('Pack name');
    expect(packIssues({ ...base(), description: 'short' }).join()).toContain('description must be');
    expect(packIssues({ ...base(), license: 'Proprietary' }).join()).toContain('license must be one of CC0-1.0, MIT, Apache-2.0, CC-BY-4.0');
    expect(packIssues({ ...base(), source: '' }).join()).toContain('source must say');
    expect(packIssues({ ...base(), verified: 'yes' }).join()).toContain('verified must be true or false');
  });
  it('checks the app and the focus rule bundle ids', () => {
    expect(packIssues({ ...base(), app: { name: 'X', bundle_id: 'not a bundle' } }).join()).toContain('Use only letters, digits');
    expect(packIssues({ ...base(), app: { name: 'X', extra: 1 } }).join()).toContain('Unknown app field "extra"');
    expect(packIssues({ ...base(), focus_rule: { bundle_id: '!' } }).join()).toContain('Use only letters, digits');
    expect(packIssues({ ...base(), focus_rule: 'yes' }).join()).toContain('focus_rule must be');
  });
  it('refuses actions a pack may not hold, and bad shortcuts', () => {
    expect(packActionIssue({ type: 'text', text: 'hello', press_enter: false })).toContain('cannot hold a "text" action');
    expect(packActionIssue({ type: 'open_url', url: 'https://example.com' })).toContain('cannot hold a "open_url" action');
    expect(packActionIssue({ type: 'shortcut', key: 'A', modifiers: [] })).toContain('is not a key');
    expect(packActionIssue({ type: 'shortcut', key: 'a', modifiers: ['command', 'command'] })).toContain('modifier twice');
    expect(packActionIssue({ type: 'shortcut', key: 'play_pause', modifiers: ['shift'] })).toContain('cannot combine');
    expect(packActionIssue({ type: 'shortcut', key: 'f13', modifiers: ['command'] })).toBeNull();
    expect(packActionIssue({ type: 'volume', delta: 0 })).toContain('nonzero');
    expect(packActionIssue({ type: 'timer', seconds: 0 })).toContain('Timer seconds');
    const forbidden = withProfile(pack => { pack.profile.bindings.push(bind({ control: 0, gesture: 'right', action: { type: 'text', text: 'x', press_enter: false } })); });
    expect(packIssues(forbidden).join()).toContain('cannot hold a "text" action');
    const tile = withProfile(pack => { pack.profile.keypad!.pages[0].slots[0] = { type: 'action', label: 'Open', bindings: [{ gesture: 'pressed', action: { type: 'open_url', url: 'https://example.com' } }] }; });
    expect(packIssues(tile).join()).toContain('cannot hold a "open_url" action');
  });
  it('refuses a profile that is not valid the way a starter must be, and one that binds too little', () => {
    expect(packIssues(withProfile(pack => { pack.profile.bindings[0].control = 40; })).join()).toContain('out of range');
    expect(packIssues(withProfile(pack => { pack.profile.keypad!.pages[0].slots.pop(); })).join()).toContain('exactly 15 slots');
    const thin = withProfile(pack => { pack.profile.bindings = [bind({})]; pack.profile.keypad!.pages[0].slots = Array(15).fill(null); });
    expect(packIssues(thin).join()).toContain('at least three controls');
  });
  it('reports at most a dozen issues', () => {
    expect(packIssues({ pack_version: 3, id: 1, name: 2, description: 3, app: 4, license: 5, source: 6, verified: 7, focus_rule: 8, profile: 9, a: 1, b: 2, c: 3 }).length).toBe(12);
  });
});

describe('parsePack and loadPacks', () => {
  it('reads valid JSON text, and explains unreadable text', () => {
    expect(parsePack(JSON.stringify(base())).pack?.id).toBe('zoom');
    const broken = parsePack('{ not json');
    expect(broken.pack).toBeNull();
    expect(broken.issues[0]).toContain('not valid JSON');
    expect(parsePack('{"id":"x"}').issues.length).toBeGreaterThan(1);
  });
  it('keeps the valid packs by name, reports each refused file, and refuses a repeated id', () => {
    const a = { ...base(), id: 'beta', name: 'Beta' };
    const b = { ...base(), id: 'alpha', name: 'Alpha' };
    const { packs, rejected } = loadPacks({
      '../presets/b.json': a, '../presets/a.json': b, '../presets/broken.json': { id: 'x' }, '../presets/dup.json': { ...base(), id: 'beta', name: 'Other' },
    });
    expect(packs.map(pack => pack.name)).toEqual(['Alpha', 'Beta']);
    expect(rejected.map(item => item.file)).toEqual(['broken.json', 'dup.json']);
    expect(rejected[1].issues).toEqual(['The id "beta" is already used by another pack.']);
  });
});
