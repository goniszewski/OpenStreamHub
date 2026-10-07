import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { Profile } from './contracts';
import { DiagnosticsPanel } from './DiagnosticsPanel';
import {
  buildDiagnosticsClient, buildDiagnosticsRequest, clampCaptureSeconds, defaultDiagnosticsOptions, maxClientBytes, maxNotesChars, runDiagnostics,
} from './diagnostics';
import type { DiagnosticsResult } from './diagnostics';
import { idleFeedState } from './device-feed';
import { emptyCounters, idleServiceState } from './service-runner';
import type { ServiceRun, ServiceStoreState } from './service-runner';

const profile: Profile = { schema_version: 1, name: 'P', bindings: [] };
const run = (id: string, texts: string[], base = 0): ServiceRun => ({
  serviceId: id, state: 'running', execute: false, entries: texts.map((text, index) => ({ kind: 'info' as const, text, at: base + index })),
  counters: emptyCounters(), startedAt: 1, profileName: 'P', snapshot: '', draftUnsaved: false,
});
const service = (runs: ServiceRun[]): ServiceStoreState => ({ ...idleServiceState, runs: Object.fromEntries(runs.map(item => [item.serviceId, item])) });
const result: DiagnosticsResult = { path: '/Users/x/diagnostics/bundle.json', bytes: 2048, summary: { devices: 2, descriptors_read: 3, descriptor_errors: 1, capture_reports: 9, redacted: true } };

describe('buildDiagnosticsClient', () => {
  it('keeps the last 200 service entries and a feed snapshot', () => {
    const client = buildDiagnosticsClient(service([run('service:a', Array.from({ length: 150 }, (_, i) => `a${i}`)), run('service:b', Array.from({ length: 150 }, (_, i) => `b${i}`), 1000)]),
      { ...idleFeedState, devices: [{ path_hex: 'aa', module: 'm131e', serial_number: 'S1' }], error: 'boom' }, '  hi  ');
    const entries = client.activity.filter(item => item.source === 'service');
    expect(entries).toHaveLength(200);
    expect(entries[entries.length - 1].text).toBe('b149');
    const feed = client.activity[client.activity.length - 1];
    expect(feed).toMatchObject({ source: 'device_feed', error: 'boom', devices: [{ serial_number: 'S1' }] });
    expect(client.notes).toBe('hi');
  });
  it('drops oldest items until under the size limit', () => {
    const big = 'x'.repeat(1900);
    const client = buildDiagnosticsClient(service([run('service:a', Array.from({ length: 200 }, (_, i) => `${i}${big}`))]), idleFeedState, '');
    expect(new TextEncoder().encode(JSON.stringify(client.activity)).length).toBeLessThan(maxClientBytes);
    expect(client.activity.length).toBeLessThan(202);
    expect(client.activity[client.activity.length - 1].source).toBe('device_feed');
    expect(client.notes).toBeNull();
  });
});

describe('buildDiagnosticsRequest', () => {
  const client = { activity: [], notes: null };
  it('uses safe defaults', () => {
    expect(buildDiagnosticsRequest(defaultDiagnosticsOptions, profile, client)).toEqual({ redact_serials: true, include_capture: false, capture_seconds: 3, include_profile: null, client });
  });
  it('clamps seconds and includes the draft', () => {
    expect(clampCaptureSeconds(0)).toBe(1); expect(clampCaptureSeconds(99)).toBe(5); expect(clampCaptureSeconds(NaN)).toBe(3);
    const request = buildDiagnosticsRequest({ ...defaultDiagnosticsOptions, redactSerials: false, includeCapture: true, captureSeconds: 9, includeProfile: true }, profile, client);
    expect(request).toMatchObject({ redact_serials: false, include_capture: true, capture_seconds: 5, include_profile: profile });
  });
  it('trims and limits notes', () => {
    const request = buildDiagnosticsRequest(defaultDiagnosticsOptions, profile, { activity: [], notes: `  ${'n'.repeat(3000)}  ` });
    expect(request.client.notes).toHaveLength(maxNotesChars);
    expect(buildDiagnosticsRequest(defaultDiagnosticsOptions, profile, { activity: [], notes: '   ' }).client.notes).toBeNull();
  });
});

describe('runDiagnostics', () => {
  it('invokes create_diagnostics with the request', async () => {
    const invoke = vi.fn().mockResolvedValue(result);
    const request = buildDiagnosticsRequest(defaultDiagnosticsOptions, profile, { activity: [], notes: null });
    expect(await runDiagnostics(invoke, request)).toEqual({ ok: true, result });
    expect(invoke).toHaveBeenCalledWith('create_diagnostics', { request });
  });
  it('reports backend errors', async () => {
    const invoke = vi.fn().mockRejectedValue('disk full');
    expect(await runDiagnostics(invoke, buildDiagnosticsRequest(defaultDiagnosticsOptions, profile, { activity: [], notes: null }))).toEqual({ ok: false, error: 'disk full' });
  });
});

describe('DiagnosticsPanel markup', () => {
  const invoke = vi.fn();
  it('renders defaults on desktop', () => {
    const html = renderToStaticMarkup(<DiagnosticsPanel desktop invoke={invoke} profile={profile} />);
    expect(html).toContain('Create diagnostic bundle');
    expect(html).toMatch(/checked=""[^>]*\/>Hide serial numbers/);
    expect(html).not.toMatch(/checked=""[^>]*\/>Include a short input capture/);
    expect(html).not.toMatch(/checked=""[^>]*\/>Include current profile/);
    expect(html).toContain('text snippets, timer labels, OBS scene names and web addresses in it would be part of the file');
    expect(html).toContain('each text snippet, timer label and OBS scene name is replaced by its length and each web address is reduced to its site address');
    expect(html).toContain('Nothing is uploaded or sent');
    expect(html).toContain('0 / 2000');
    expect(html).toContain('<option value="3" selected="">3 seconds</option>');
    expect(html).not.toContain('role="alert"');
  });
  it('is disabled in the browser preview', () => {
    const html = renderToStaticMarkup(<DiagnosticsPanel desktop={false} invoke={invoke} profile={profile} />);
    expect(html).toContain('needs the desktop app');
    expect(html).toMatch(/<button type="button" class="button primary" disabled=""/);
  });
  it('shows the result card', () => {
    const html = renderToStaticMarkup(<DiagnosticsPanel desktop invoke={invoke} profile={profile} initialResult={result} />);
    expect(html).toContain('/Users/x/diagnostics/bundle.json');
    expect(html).toContain('2.0 KiB');
    expect(html).toContain('Copy path');
    expect(html).toContain('attach the file manually');
  });
});
