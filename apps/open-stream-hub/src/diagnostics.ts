import type { Profile } from './contracts';
import type { FeedStoreState } from './device-feed';
import type { Invoke } from './profile-session';
import type { ServiceStoreState } from './service-runner';
import { errorMessage } from './ui-model';

export const maxNotesChars = 2000;
export const maxClientActivity = 200;
export const maxClientBytes = 200 * 1024;
const maxTextChars = 2000;
export const minCaptureSeconds = 1;
export const maxCaptureSeconds = 5;
export const defaultCaptureSeconds = 3;

export interface DiagnosticsClient { activity: Record<string, unknown>[]; notes: string | null }
export interface DiagnosticsOptions { redactSerials: boolean; includeCapture: boolean; captureSeconds: number; includeProfile: boolean; notes: string }
export const defaultDiagnosticsOptions: DiagnosticsOptions = { redactSerials: true, includeCapture: false, captureSeconds: defaultCaptureSeconds, includeProfile: false, notes: '' };

export interface DiagnosticsRequest {
  redact_serials: boolean;
  include_capture: boolean;
  capture_seconds: number;
  include_profile: Profile | null;
  client: DiagnosticsClient;
}
export interface DiagnosticsResult {
  path: string;
  bytes: number;
  summary: { devices: number; descriptors_read: number; descriptor_errors: number; capture_reports: number; redacted: boolean };
}

const clip = (text: string) => text.length > maxTextChars ? `${text.slice(0, maxTextChars)}…` : text;
const byteLength = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length;

export const clampCaptureSeconds = (value: number) =>
  Number.isFinite(value) ? Math.min(maxCaptureSeconds, Math.max(minCaptureSeconds, Math.round(value))) : defaultCaptureSeconds;

export function normalizeNotes(notes: string): string | null {
  const trimmed = notes.trim().slice(0, maxNotesChars).trim();
  return trimmed === '' ? null : trimmed;
}

// Oldest-first activity: the last 200 service entries across all runs, then
// one snapshot of the device feed. Dropping from the front keeps the newest.
export function buildDiagnosticsClient(service: ServiceStoreState, feed: FeedStoreState, notes: string): DiagnosticsClient {
  const entries: Record<string, unknown>[] = [];
  for (const run of Object.values(service.runs)) {
    for (const entry of run.entries) {
      entries.push({
        source: 'service', service_id: run.serviceId, state: run.state, execute: run.execute, profile_name: run.profileName,
        kind: entry.kind, text: clip(entry.text), at: entry.at,
      });
    }
  }
  entries.sort((a, b) => (a.at as number) - (b.at as number));
  const activity = entries.slice(-maxClientActivity);
  for (const run of Object.values(service.runs)) {
    activity.push({
      source: 'service_status', service_id: run.serviceId, state: run.state, execute: run.execute, exit_code: run.exitCode ?? null,
      error: run.error ? clip(run.error) : null, stderr_tail: run.stderrTail ? clip(run.stderrTail) : null, counters: run.counters, started_at: run.startedAt,
    });
  }
  if (service.error) activity.push({ source: 'service_store', error: clip(service.error) });
  activity.push({
    source: 'device_feed', auto_detect: feed.autoDetect, error: feed.error ? clip(feed.error) : null,
    devices: feed.devices.map(device => ({ path_hex: device.path_hex, module: device.module, serial_number: device.serial_number })),
    feeds: Object.values(feed.feeds).map(status => ({ feed_id: status.feedId, state: status.state, source: status.source, module: status.module, error: status.error ? clip(status.error) : null })),
  });
  while (activity.length > 0 && byteLength(activity) >= maxClientBytes) activity.shift();
  return { activity, notes: normalizeNotes(notes) };
}

export function buildDiagnosticsRequest(options: DiagnosticsOptions, draft: Profile, client: DiagnosticsClient): DiagnosticsRequest {
  return {
    redact_serials: options.redactSerials,
    include_capture: options.includeCapture,
    capture_seconds: clampCaptureSeconds(options.captureSeconds),
    include_profile: options.includeProfile ? draft : null,
    client: { activity: client.activity, notes: normalizeNotes(client.notes ?? '') },
  };
}

export type DiagnosticsOutcome = { ok: true; result: DiagnosticsResult } | { ok: false; error: string };

export async function runDiagnostics(invoke: Invoke, request: DiagnosticsRequest): Promise<DiagnosticsOutcome> {
  try {
    return { ok: true, result: await invoke<DiagnosticsResult>('create_diagnostics', { request }) };
  } catch (reason) {
    return { ok: false, error: errorMessage(reason) };
  }
}

export const formatBytes = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KiB` : `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
