// Pure helpers for duplicating, importing and exporting profiles as local JSON.
import type { Profile } from './contracts';
import { clampUtf8, maximumLabelBytes } from './draft-model';
import { upgradeKeypadProfile } from './keypad-model';

export const maximumImportBytes = 1024 * 1024;

const encoder = new TextEncoder();
const taken = (names: readonly string[], candidate: string) => names.some(name => name.trim().toLowerCase() === candidate.trim().toLowerCase());

// `<base><suffix>` within the label byte limit; the base is shortened, not the suffix.
function withSuffix(name: string, suffix: string) {
  return clampUtf8(name, maximumLabelBytes - encoder.encode(suffix).length) + suffix;
}

function firstFree(name: string, existing: readonly string[], suffixes: Iterable<string>) {
  for (const suffix of suffixes) {
    const candidate = withSuffix(name, suffix);
    if (!taken(existing, candidate)) return candidate;
  }
  throw new Error('Could not find an unused profile name.');
}

function* copySuffixes() {
  yield ' copy';
  for (let count = 2; count < 10_000; count += 1) yield ` copy ${count}`;
}

// `<name> copy`, `<name> copy 2`, ...
export function copyName(name: string, existing: readonly string[]) {
  return firstFree(name, existing, copySuffixes());
}

// `<name>`, then `<name> (imported)`, `<name> (imported 2)`, ...
export function importName(name: string, existing: readonly string[]) {
  return firstFree(name, existing, importSuffixes());
}

function* importSuffixes() {
  yield '';
  yield ' (imported)';
  for (let count = 2; count < 10_000; count += 1) yield ` (imported ${count})`;
}

export function withName(profile: Profile, name: string): Profile {
  return { ...profile, name };
}

export class ImportError extends Error {}

export function checkImportSize(bytes: number) {
  if (bytes > maximumImportBytes) throw new ImportError(`This file is ${(bytes / 1024 / 1024).toFixed(1)} MiB. Profile files must be at most 1 MiB.`);
}

// Parses and upgrades only; service validation is a separate backend call.
// Nothing in the file is ever executed.
export function parseImportedProfile(text: string): Profile {
  checkImportSize(encoder.encode(text).length);
  let value: unknown;
  try { value = JSON.parse(text); } catch { throw new ImportError('This file is not valid JSON.'); }
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new ImportError('This file is not a profile: expected a JSON object.');
  const candidate = value as Partial<Profile>;
  if (typeof candidate.name !== 'string' || !Array.isArray(candidate.bindings)) throw new ImportError('This file is not a profile: it needs a name and a bindings list.');
  try { return upgradeKeypadProfile(candidate as Profile); }
  catch (reason) { throw new ImportError(reason instanceof Error ? reason.message : 'This profile version is not supported.'); }
}

// Reads a user-chosen file in the webview after checking its size.
export async function readImportFile(file: Pick<File, 'size' | 'text'>): Promise<string> {
  checkImportSize(file.size);
  return file.text();
}

// Official MasterHUB files: a .mhz, or the .zip of several .mhz files the official app exports. The Rust side enforces
// the same limit and every archive rule again; nothing in the file is ever executed.
export const maximumMhzBytes = 32 * 1024 * 1024;
export const isMhzFileName = (name: string) => /\.(mhz|zip)$/i.test(name.trim());

export async function readMhzFile(file: Pick<File, 'size' | 'arrayBuffer'>): Promise<Uint8Array> {
  if (file.size > maximumMhzBytes) throw new ImportError(`This file is ${(file.size / 1024 / 1024).toFixed(1)} MiB. Official profile files must be at most 32 MiB.`);
  return new Uint8Array(await file.arrayBuffer());
}
