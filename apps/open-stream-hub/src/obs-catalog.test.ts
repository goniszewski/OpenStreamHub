import { describe, expect, it } from 'vitest';
import fixture from '../fixtures/obs-catalog.json';
import type { ObsCatalogNames } from './contracts';
import { catalogNames, filterTargets, idleObsCatalogState, isObsCatalogNames, ObsCatalogStore } from './obs-catalog';

// The listing replies are the shared fixture from src-tauri (contract_fixtures.rs).
describe('OBS catalog contract', () => {
  it('matches the Rust fixture', () => {
    expect(isObsCatalogNames(fixture.obs_catalog_result)).toBe(true);
    expect(Object.keys(fixture.obs_source_filters_args)).toEqual(['source']);
    expect(Object.keys(fixture.obs_scene_sources_args)).toEqual(['scene']);
    expect(fixture.obs_source_filters_result).toEqual(['Blur']);
  });
  it('accepts only the exact catalog shape', () => {
    for (const bad of [null, 'catalog', {}, { scenes: [] }, { scenes: [], sources: [], scene_collections: [], profiles: [], extra: [] },
      { scenes: 'Scene', sources: [], scene_collections: [], profiles: [] },
      { scenes: [5], sources: [], scene_collections: [], profiles: [] },
      { scenes: Array.from({ length: 1001 }, (_, index) => `S${index}`), sources: [], scene_collections: [], profiles: [] }]) {
      expect(isObsCatalogNames(bad), JSON.stringify(bad)?.slice(0, 60)).toBe(false);
    }
  });
});

describe('catalog helpers', () => {
  const names: ObsCatalogNames = { scenes: ['Games', 'Live'], sources: ['Cam', 'Mic'], scene_collections: ['Show'], profiles: [] };
  it('offer the catalog lists only once they arrived', () => {
    expect(catalogNames(undefined, undefined)).toBe(null);
    expect(catalogNames({ ...idleObsCatalogState }, undefined)).toBe(null);
    expect(catalogNames({ ...idleObsCatalogState, status: 'loading' }, undefined)).toEqual({ status: 'loading' });
    // A failed refresh keeps the last list offered: still real names, and the
    // connection note says the listing could not be renewed.
    expect(catalogNames({ ...idleObsCatalogState, status: 'failed', error: 'x' }, names.scenes)).toEqual({ status: 'ready', names: ['Games', 'Live'] });
    expect(catalogNames({ ...idleObsCatalogState, status: 'ready', names }, names.scenes)).toEqual({ status: 'ready', names: ['Games', 'Live'] });
  });
  it('offer inputs and scenes to a filter, in one case-insensitive order', () => {
    expect(filterTargets(undefined)).toBe(null);
    expect(filterTargets({ ...idleObsCatalogState, names })).toEqual({
      status: 'ready',
      names: ['Cam', 'Games', 'Live', 'Mic'],
    });
  });
});

describe('ObsCatalogStore', () => {
  const catalog: ObsCatalogNames = {
    scenes: ['live', 'Games', 'live'],
    sources: ['Mic', 'cam 10', 'cam 2'],
    scene_collections: ['Show'],
    profiles: ['Streaming', 'Desk'],
  };

  type Call = [string, Record<string, unknown> | undefined];
  const storeWith = (reply: (command: string, args?: Record<string, unknown>) => unknown, desktop = true) => {
    const calls: Call[] = [];
    const store = new ObsCatalogStore(async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      return reply(command, args) as T;
    }, desktop);
    return { store, calls };
  };

  it('asks once, sorts, de-duplicates and drops per-name lists on refresh', async () => {
    const { store, calls } = storeWith(() => catalog);
    expect(store.getState().status).toBe('idle');
    expect(await store.refresh()).toBe(true);
    expect(calls).toEqual([['obs_catalog', undefined]]);
    expect(store.getState().names).toEqual({
      scenes: ['Games', 'live'],
      sources: ['cam 2', 'cam 10', 'Mic'],
      scene_collections: ['Show'],
      profiles: ['Desk', 'Streaming'],
    });
    // A second press asks again; the store never caches the catalog itself.
    expect(await store.refresh()).toBe(true);
    expect(calls).toHaveLength(2);
  });

  it('stays idle outside the desktop app and never asks Tauri', async () => {
    const { store, calls } = storeWith(() => catalog, false);
    expect(await store.refresh()).toBe(false);
    store.requestNames('filters', 'Cam');
    expect(calls).toEqual([]);
    expect(store.getState().status).toBe('idle');
    expect(store.list('filters', 'Cam')).toBe(null);
  });

  it('reports why OBS could not be asked and keeps the fields plain', async () => {
    const { store } = storeWith(() => { throw new Error('OBS at 127.0.0.1:4455: could not connect'); });
    expect(await store.refresh()).toBe(false);
    const state = store.getState();
    expect(state.status).toBe('failed');
    expect(state.error).toContain('could not connect');
    expect(state.names).toBe(null);
  });

  it('refuses a reply that is not a catalog', async () => {
    const { store } = storeWith(() => ({ scenes: 'Live' }));
    expect(await store.refresh()).toBe(false);
    expect(store.getState().error).toContain('not understood');
  });

  it('fetches a per-name list on request, caches it and records a failure', async () => {
    const { store, calls } = storeWith(() => ['Blur', 'Chroma', 'Blur']);
    await store.refresh();
    calls.length = 0;
    expect(store.list('filters', 'Cam')).toBe(null);
    store.requestNames('filters', 'Cam');
    expect(store.list('filters', 'Cam')).toEqual({ status: 'loading' });
    await Promise.resolve();
    await Promise.resolve();
    expect(calls).toEqual([['obs_source_filters', { source: 'Cam' }]]);
    expect(store.list('filters', 'Cam')).toEqual({ status: 'ready', names: ['Blur', 'Chroma'] });
    // A second request for the same name does not ask again; another name does.
    store.requestNames('filters', 'Cam');
    store.requestNames('sceneSources', 'Live');
    await Promise.resolve();
    await Promise.resolve();
    expect(calls).toEqual([
      ['obs_source_filters', { source: 'Cam' }],
      ['obs_scene_sources', { scene: 'Live' }],
    ]);
    expect(store.list('sceneSources', 'Live')).toEqual({ status: 'ready', names: ['Blur', 'Chroma'] });

    const failing = new ObsCatalogStore(async () => { throw new Error('OBS refused GetSourceFilterList (no such source)'); }, true);
    await failing.refresh();
    failing.requestNames('filters', 'Ghost');
    await Promise.resolve();
    await Promise.resolve();
    expect(failing.list('filters', 'Ghost')).toEqual({ status: 'failed', error: 'OBS refused GetSourceFilterList (no such source)' });
  });

  it('ignores per-name requests it cannot answer', async () => {
    const { store, calls } = storeWith(() => []);
    await store.refresh();
    calls.length = 0;
    store.requestNames('filters', '');
    store.requestNames('filters', '   ');
    store.requestNames('filters', 'x'.repeat(257));
    expect(calls).toEqual([]);
    // An empty list is a real answer, never re-requested.
    store.requestNames('filters', 'Cam');
    await Promise.resolve();
    await Promise.resolve();
    expect(store.list('filters', 'Cam')).toEqual({ status: 'ready', names: [] });
    calls.length = 0;
    store.requestNames('filters', 'Cam');
    expect(calls).toEqual([]);
  });

  it('drops stale per-name lists when the catalog is asked again', async () => {
    const { store } = storeWith((_command, args) => args ? [] : catalog);
    await store.refresh();
    store.requestNames('filters', 'Cam');
    await Promise.resolve();
    await Promise.resolve();
    expect(store.list('filters', 'Cam')).toEqual({ status: 'ready', names: [] });
    await store.refresh();
    expect(store.list('filters', 'Cam')).toBe(null);
    expect(store.getState().names?.scenes).toEqual(['Games', 'live']);
  });

  it('seeds sample listings in the browser preview only', () => {
    const { store } = storeWith(() => catalog, false);
    store.seedPreview(catalog);
    const state = store.getState();
    expect(state.status).toBe('ready');
    expect(state.names?.sources).toEqual(['cam 2', 'cam 10', 'Mic']);
    expect(store.list('filters', 'Cam')).toBe(null);
  });
});
