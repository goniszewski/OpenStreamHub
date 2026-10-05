import { describe, expect, it, vi } from 'vitest';
import type { Profile } from './contracts';
import { draftKeyPrefix, DraftPersister, DraftStore, draftWriteDelayMs, maximumDraftBytes, newDraftKey, restoreDecision } from './draft-store';
import { upgradeKeypadProfile } from './keypad-model';
import { FakeStorage } from './fake-storage';

const profile = (name: string): Profile => upgradeKeypadProfile({ schema_version: 1, name, bindings: [] });
const broken = (): Storage => new Proxy({}, { get() { throw new Error('storage unavailable'); } }) as Storage;

function scheduler() {
  const tasks: { action: () => void; delay: number; cancelled: boolean }[] = [];
  const schedule = (action: () => void, delay: number) => { const task = { action, delay, cancelled: false }; tasks.push(task); return () => { task.cancelled = true; }; };
  const run = () => tasks.filter(task => !task.cancelled).forEach(task => { task.cancelled = true; task.action(); });
  return { schedule, tasks, run };
}

describe('DraftStore', () => {
  it('round-trips a draft with its base time', () => {
    const storage = new FakeStorage();
    const store = new DraftStore(storage, () => 42);
    expect(store.write('a', profile('A'), 7)).toBe('written');
    expect(store.read('a')).toEqual({ profile: profile('A'), base_modified_at_ms: 7, saved_at: 42 });
    expect(storage.getItem(`${draftKeyPrefix}a`)).not.toBeNull();
    store.clear('a');
    expect(store.read('a')).toBeNull();
  });

  it('refuses drafts over the size cap and removes the stale copy', () => {
    const storage = new FakeStorage();
    const store = new DraftStore(storage);
    store.write('a', profile('small'), 1);
    const huge = { ...profile('big'), name: 'x'.repeat(maximumDraftBytes) };
    expect(store.write('a', huge, 1)).toBe('too-large');
    expect(store.read('a')).toBeNull();
  });

  it('drops corrupt or malformed entries', () => {
    const storage = new FakeStorage();
    const store = new DraftStore(storage);
    storage.setItem(`${draftKeyPrefix}a`, '{nope');
    storage.setItem(`${draftKeyPrefix}b`, JSON.stringify({ profile: { name: 1 }, base_modified_at_ms: 1, saved_at: 1 }));
    expect(store.read('a')).toBeNull();
    expect(store.read('b')).toBeNull();
    expect(storage.length).toBe(0);
  });

  it('purges drafts of profiles that no longer exist but keeps the new-profile draft and other keys', () => {
    const storage = new FakeStorage();
    const store = new DraftStore(storage);
    store.write('a', profile('A'), 1); store.write('gone', profile('G'), 1); store.write(newDraftKey, profile('N'), null);
    storage.setItem('appearance', 'dark');
    store.purge(['a']);
    expect(store.keys().sort()).toEqual(['a', newDraftKey]);
    expect(storage.getItem('appearance')).toBe('dark');
  });

  it('works without storage and with storage that throws', () => {
    for (const storage of [null, broken()]) {
      const store = new DraftStore(storage);
      expect(store.write('a', profile('A'), 1)).toBe('unavailable');
      expect(store.read('a')).toBeNull();
      expect(store.keys()).toEqual([]);
      expect(() => { store.clear('a'); store.purge(['b']); }).not.toThrow();
    }
  });
});

describe('restoreDecision', () => {
  const stored = { profile: profile('Edited'), base_modified_at_ms: 10, saved_at: 1 };
  it('offers nothing without a stored draft or when it equals the saved profile', () => {
    expect(restoreDecision(null, { profile: profile('A'), modifiedAtMs: 10 })).toEqual({ kind: 'none' });
    expect(restoreDecision(stored, { profile: profile('Edited'), modifiedAtMs: 99 })).toEqual({ kind: 'none' });
  });
  it('flags a profile that changed on disk since the draft was saved', () => {
    expect(restoreDecision(stored, { profile: profile('A'), modifiedAtMs: 10 })).toEqual({ kind: 'offer', changedOnDisk: false });
    expect(restoreDecision(stored, { profile: profile('A'), modifiedAtMs: 11 })).toEqual({ kind: 'offer', changedOnDisk: true });
    expect(restoreDecision({ ...stored, base_modified_at_ms: null }, null)).toEqual({ kind: 'offer', changedOnDisk: false });
  });
});

describe('DraftPersister', () => {
  const setup = () => {
    const storage = new FakeStorage();
    const store = new DraftStore(storage, () => 1);
    const timer = scheduler();
    const persister = new DraftPersister(store, timer.schedule);
    return { storage, store, timer, persister };
  };
  const dirty = (key: string, name: string, base: number | null = 5) => ({ key, profile: profile(name), baseModifiedAtMs: base, dirty: true });

  it('debounces writes and keeps only the latest', () => {
    const { store, timer, persister } = setup();
    persister.observe(dirty('a', 'One'));
    persister.observe(dirty('a', 'Two'));
    expect(timer.tasks.every(task => task.delay === draftWriteDelayMs)).toBe(true);
    expect(store.read('a')).toBeNull();
    timer.run();
    expect(store.read('a')?.profile.name).toBe('Two');
  });

  it('flushes pending writes immediately (page hide) and does nothing when idle', () => {
    const { store, persister } = setup();
    persister.flush();
    persister.observe(dirty('a', 'One'));
    persister.flush();
    expect(store.read('a')?.profile.name).toBe('One');
  });

  it('clears when the draft equals the saved profile, and cancels the pending write', () => {
    const { store, timer, persister } = setup();
    persister.observe(dirty('a', 'One')); persister.flush();
    persister.observe(dirty('a', 'Two'));
    persister.observe({ ...dirty('a', 'Saved'), dirty: false });
    timer.run();
    expect(store.read('a')).toBeNull();
  });

  it('clears the previous profile draft when the open profile changes', () => {
    const { store, persister } = setup();
    persister.observe(dirty('a', 'One')); persister.flush();
    persister.observe({ ...dirty('b', 'B'), dirty: false });
    expect(store.read('a')).toBeNull();
  });

  it('clears on explicit discard', () => {
    const { store, persister } = setup();
    persister.observe(dirty('a', 'One')); persister.flush();
    persister.discardCurrent();
    expect(store.read('a')).toBeNull();
  });

  it('leaves a held draft alone until released', () => {
    const { store, timer, persister } = setup();
    store.write('a', profile('Crashed'), 1);
    persister.hold('a');
    persister.observe({ ...dirty('a', 'Saved'), dirty: false });
    persister.observe(dirty('a', 'Typed'));
    timer.run(); persister.flush();
    expect(store.read('a')?.profile.name).toBe('Crashed');
    persister.release('a');
    persister.observe(dirty('a', 'Typed'));
    persister.flush();
    expect(store.read('a')?.profile.name).toBe('Typed');
  });

  it('reports an oversized draft once', () => {
    const { store, persister } = setup();
    const oversize = vi.fn();
    persister.onOversize = oversize;
    const huge = { ...profile('big'), name: 'x'.repeat(maximumDraftBytes) };
    persister.observe({ key: 'a', profile: huge, baseModifiedAtMs: 1, dirty: true }); persister.flush();
    persister.observe({ key: 'a', profile: { ...huge, name: huge.name + 'y' }, baseModifiedAtMs: 1, dirty: true }); persister.flush();
    expect(oversize).toHaveBeenCalledOnce();
    expect(store.read('a')).toBeNull();
  });

  it('keeps working when storage is unavailable', () => {
    const persister = new DraftPersister(new DraftStore(broken()), scheduler().schedule);
    expect(() => { persister.observe(dirty('a', 'One')); persister.flush(); persister.discardCurrent(); }).not.toThrow();
  });
});
