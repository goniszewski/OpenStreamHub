#!/usr/bin/env node
// Builds `masterhub-service` and copies it to src-tauri/binaries/ under the
// name Tauri's `bundle.externalBin` expects: masterhub-service-<target-triple>.
// Usage: node scripts/prepare-sidecar.mjs [--release] [--target <triple>]
// Env: TAURI_ENV_TARGET_TRIPLE (set by the Tauri CLI), CARGO_TARGET_DIR.
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const BIN = 'masterhub-service';

export function parseHostTriple(rustcVersionOutput) {
  const line = rustcVersionOutput.split('\n').find((entry) => entry.startsWith('host:'));
  const triple = line?.slice('host:'.length).trim();
  if (!triple) throw new Error('could not find a "host:" line in `rustc -vV` output');
  return triple;
}

export function exeSuffix(triple) {
  return triple.includes('windows') ? '.exe' : '';
}

export function sidecarFileName(triple) {
  return `${BIN}-${triple}${exeSuffix(triple)}`;
}

/** Where cargo writes the binary. `targetOverride` is only set for explicit --target builds. */
export function builtBinaryPath({ workspaceRoot, targetDir, triple, targetOverride, release }) {
  const base = targetDir
    ? isAbsolute(targetDir) ? targetDir : resolve(workspaceRoot, targetDir)
    : join(workspaceRoot, 'target');
  return join(base, ...(targetOverride ? [triple] : []), release ? 'release' : 'debug', `${BIN}${exeSuffix(triple)}`);
}

export function parseArgs(argv, env = {}) {
  let release = false;
  let target = env.TAURI_ENV_TARGET_TRIPLE || undefined;
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--release') release = true;
    else if (argv[i] === '--target') {
      target = argv[i + 1];
      i += 1;
      if (!target) throw new Error('--target needs a value');
    } else throw new Error(`unknown argument: ${argv[i]}`);
  }
  return { release, target };
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options });
  if (result.error?.code === 'ENOENT') {
    throw new Error(`\`${command}\` was not found on PATH. Install Rust (https://rustup.rs) to build the ${BIN} sidecar.`);
  }
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`\`${command} ${args.join(' ')}\` failed with status ${result.status}`);
  return result;
}

function sameContent(a, b) {
  return existsSync(b) && statSync(a).size === statSync(b).size && readFileSync(a).equals(readFileSync(b));
}

function main() {
  const here = dirname(fileURLToPath(import.meta.url));
  const appDir = resolve(here, '..');
  const workspaceRoot = resolve(appDir, '../..');
  const { release, target } = parseArgs(process.argv.slice(2), process.env);
  const triple = target ?? parseHostTriple(run('rustc', ['-vV']).stdout);

  const cargoArgs = ['build', '-p', 'cooltek-sdk-cli', '--bin', BIN, '--locked'];
  if (release) cargoArgs.push('--release');
  if (target) cargoArgs.push('--target', target);
  run('cargo', cargoArgs, { cwd: workspaceRoot, stdio: 'inherit' });

  const built = builtBinaryPath({
    workspaceRoot,
    targetDir: process.env.CARGO_TARGET_DIR,
    triple,
    targetOverride: Boolean(target),
    release,
  });
  if (!existsSync(built)) throw new Error(`cargo succeeded but ${built} does not exist`);

  const destination = join(appDir, 'src-tauri', 'binaries', sidecarFileName(triple));
  mkdirSync(dirname(destination), { recursive: true });
  const unchanged = sameContent(built, destination);
  if (!unchanged) copyFileSync(built, destination);
  chmodSync(destination, 0o755);
  console.log(`sidecar: ${unchanged ? 'up to date' : 'copied'} ${destination} (${release ? 'release' : 'debug'}, ${triple})`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`sidecar: ${error.message}`);
    process.exit(1);
  }
}
