// A sample manifest and listing shared by the plugin tests.
import type { PluginListing, PluginManifest, PluginManifestAction } from './plugins-model';

export const lights: PluginManifest & { actions: PluginManifestAction[] } = {
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

// The same plugin with conditions an if / else can test, and a plugin that has only conditions.
export const checkingLights: PluginManifest = {
  ...lights,
  conditions: [
    { id: 'is_on', label: 'Light is on', params: [
      { id: 'entity', label: 'Light', type: 'text', variables: true, required: true },
      { id: 'token', label: 'Token', type: 'text', secret: true },
    ] },
    { id: 'reachable', label: 'Hub is reachable' },
  ],
};
export const meetings: PluginManifest = {
  schema: 1, id: 'com.example.meetings', name: 'Meetings', version: '2.0.0', command: ['bin/meetings'],
  conditions: [{ id: 'in_call', label: 'A call is running' }],
};

export const conditionListing: PluginListing = {
  folder: listing.folder,
  plugins: [
    { folder: 'com.example.lights', manifest: checkingLights, fingerprint: 'f2', approval: 'enabled', secrets: [{ id: 'token', label: 'Token', set: true }] },
    { folder: 'com.example.meetings', manifest: meetings, fingerprint: 'm1', approval: 'enabled' },
    { folder: 'com.example.hall', manifest: { ...checkingLights, id: 'com.example.hall', name: 'Hall' }, fingerprint: 'h1', approval: 'not_enabled' },
  ],
};
