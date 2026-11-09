// A sample manifest and listing shared by the plugin tests.
import type { PluginListing, PluginManifest } from './plugins-model';

export const lights: PluginManifest = {
  schema: 1, id: 'com.example.lights', name: 'Lights', version: '1.2.0', command: ['bin/lights'],
  actions: [
    { id: 'toggle', label: 'Toggle' },
    { id: 'set', label: 'Set a light', params: [
      { id: 'entity', label: 'Light', type: 'text', variables: true, required: true },
      { id: 'level', label: 'Level', type: 'number', min: 0, max: 100, default: 50 },
      { id: 'fade', label: 'Fade', type: 'bool' },
      { id: 'mode', label: 'Mode', type: 'choice', choices: [{ id: 'warm', label: 'Warm' }, { id: 'cold', label: 'Cold' }], default: 'warm' },
      { id: 'token', label: 'Token', type: 'text', secret: true },
    ] },
  ],
  permissions: { variables: ['room'] },
};

export const listing: PluginListing = {
  folder: '/Users/me/Library/Application Support/app/plugins',
  plugins: [
    { folder: 'com.example.broken', error: 'plugin.json is not a valid manifest' },
    { folder: 'com.example.lights', manifest: lights, fingerprint: 'f1', approval: 'enabled', secrets: [{ id: 'token', label: 'Token', set: true }] },
  ],
};
