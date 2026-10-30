import { describe, expect, it } from 'vitest';
import fixture from '../fixtures/streamlabs-catalog.json';
import type { StreamlabsCatalogNames } from './contracts';
import { idleCatalogState } from './obs-catalog';
import { isStreamlabsCatalogNames, StreamlabsCatalogStore, streamlabsSceneNames } from './streamlabs-catalog';
import type { StreamlabsCatalogView } from './streamlabs-catalog';

// The listing replies are the shared fixture from src-tauri (contract_fixtures.rs).
describe('Streamlabs catalog contract', () => {
  it('matches the Rust fixture', () => {
    expect(isStreamlabsCatalogNames(fixture.streamlabs_catalog_result)).toBe(true);
    expect(isStreamlabsCatalogNames(fixture.streamlabs_catalog_without_active_result)).toBe(true);
    expect(Object.keys(fixture.streamlabs_scene_sources_args)).toEqual(['scene']);
    expect(fixture.streamlabs_scene_sources_result).toEqual(['Camera', 'Overlays', 'Logo']);
  });
  it('accepts only the exact catalog shape', () => {
    const good = { scenes: [], scene_collections: [], active_collection: null, audio_sources: [] };
    for (const bad of [null, 'catalog', {}, { ...good, extra: [] }, { scenes: [], scene_collections: [], audio_sources: [] },
      { ...good, active_collection: 5 }, { ...good, audio_sources: [5] }, { ...good, scenes: 'Live' },
      { ...good, scenes: Array.from({ length: 1001 }, (_, index) => `S${index}`) },
      // The OBS shape is not a Streamlabs catalog.
      { scenes: [], sources: [], scene_collections: [], profiles: [] }]) {
      expect(isStreamlabsCatalogNames(bad), JSON.stringify(bad)?.slice(0, 60)).toBe(false);
    }
    expect(isStreamlabsCatalogNames({ ...good, active_collection: 'Main' })).toBe(true);
  });
});

describe('StreamlabsCatalogStore', () => {
  const catalog: StreamlabsCatalogNames = {
    scenes: ['live', 'Be right back', 'live'], scene_collections: ['Podcast', 'Main'], active_collection: 'Main', audio_sources: ['Mic 10', 'Mic 2'],
  };
  type Call = [string, Record<string, unknown> | undefined];
  const storeWith = (reply: (command: string, args?: Record<string, unknown>) => unknown, desktop = true) => {
    const calls: Call[] = [];
    const store = new StreamlabsCatalogStore(async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      return reply(command, args) as T;
    }, desktop);
    return { store, calls };
  };
  const settle = async () => { await Promise.resolve(); await Promise.resolve(); };

  it('asks once, sorts, de-duplicates and marks the listing connected', async () => {
    const { store, calls } = storeWith(() => catalog);
    expect(await store.refresh()).toBe(true);
    expect(calls).toEqual([['streamlabs_catalog', undefined]]);
    expect(store.getState().names).toEqual({ scenes: ['Be right back', 'live'], scene_collections: ['Main', 'Podcast'], active_collection: 'Main', audio_sources: ['Mic 2', 'Mic 10'] });
    expect(store.getState().connection).toEqual({ state: 'connected', message: 'Connected to Streamlabs Desktop.' });
  });

  it('asks the backend why a failed listing failed and keeps the fields plain', async () => {
    const verdict = { state: 'token_needed', message: 'Streamlabs asks for an API token.' };
    const { store, calls } = storeWith(command => { if (command === 'streamlabs_status') return verdict; throw new Error('Streamlabs has no API token in OpenStreamHub Settings'); });
    expect(await store.refresh()).toBe(false);
    expect(calls.map(call => call[0])).toEqual(['streamlabs_catalog', 'streamlabs_status']);
    expect(store.getState()).toMatchObject({ status: 'failed', names: null, connection: verdict, error: 'Streamlabs has no API token in OpenStreamHub Settings' });
    // An OBS verdict is not a Streamlabs one.
    const odd = storeWith(command => { if (command === 'streamlabs_status') return { state: 'websocket_off', message: 'x' }; throw new Error('boom'); });
    await odd.store.refresh();
    expect(odd.store.getState()).toMatchObject({ status: 'failed', connection: null });
    const unreadable = storeWith(() => ({ scenes: 'Live' }));
    await unreadable.store.refresh();
    expect(unreadable.store.getState().error).toBe('The Streamlabs listing reply was not understood.');
  });

  it('fetches the items of a scene on request and never asks outside the desktop app', async () => {
    const { store, calls } = storeWith(command => command === 'streamlabs_catalog' ? catalog : ['Logo', 'Camera', 'Logo']);
    await store.refresh();
    calls.length = 0;
    store.requestNames('sceneSources', 'Live');
    expect(store.list('sceneSources', 'Live')).toEqual({ status: 'loading' });
    await settle();
    expect(calls).toEqual([['streamlabs_scene_sources', { scene: 'Live' }]]);
    expect(store.list('sceneSources', 'Live')).toEqual({ status: 'ready', names: ['Camera', 'Logo'] });
    store.requestNames('sceneSources', 'Live');
    store.requestNames('sceneSources', ' ');
    store.requestNames('sceneSources', 'x'.repeat(129));
    await settle();
    expect(calls).toHaveLength(1);

    const preview = storeWith(() => catalog, false);
    expect(await preview.store.refresh()).toBe(false);
    preview.store.requestNames('sceneSources', 'Live');
    expect(preview.calls).toEqual([]);
  });
});

describe('streamlabsSceneNames', () => {
  const view = (names: StreamlabsCatalogNames | null, status: StreamlabsCatalogView['status'] = 'ready'): StreamlabsCatalogView => ({
    ...idleCatalogState, desktop: true, status, names, refresh: () => {}, list: () => null, requestNames: () => {},
  });
  const names: StreamlabsCatalogNames = { scenes: ['Live'], scene_collections: ['Main', 'Podcast'], active_collection: 'Main', audio_sources: [] };
  it('offers the active collection\'s scenes only for a switch within it', () => {
    expect(streamlabsSceneNames(undefined, '')).toBe(null);
    expect(streamlabsSceneNames(view(names), '')).toEqual({ status: 'ready', names: ['Live'] });
    expect(streamlabsSceneNames(view(names), 'Main')).toEqual({ status: 'ready', names: ['Live'] });
    // Another collection's scenes are not listed, so its field stays plain instead of warning.
    expect(streamlabsSceneNames(view(names), 'Podcast')).toBe(null);
    expect(streamlabsSceneNames(view({ ...names, active_collection: null }), 'Main')).toBe(null);
    expect(streamlabsSceneNames(view(null, 'loading'), '')).toEqual({ status: 'loading' });
  });
});
