import { describe, expect, it } from 'vitest';
// @ts-expect-error plain ESM script without type declarations
import * as sidecar from './prepare-sidecar.mjs';

describe('prepare-sidecar helpers', () => {
  it('parses the host triple from rustc -vV', () => {
    const out = 'rustc 1.97.0\nbinary: rustc\nhost: aarch64-apple-darwin\nrelease: 1.97.0\n';
    expect(sidecar.parseHostTriple(out)).toBe('aarch64-apple-darwin');
    expect(() => sidecar.parseHostTriple('nothing')).toThrow(/host:/);
  });

  it('names sidecars with the triple and a windows suffix', () => {
    expect(sidecar.sidecarFileName('aarch64-apple-darwin')).toBe('masterhub-service-aarch64-apple-darwin');
    expect(sidecar.sidecarFileName('x86_64-pc-windows-msvc')).toBe('masterhub-service-x86_64-pc-windows-msvc.exe');
  });

  it('computes cargo output paths', () => {
    const base = { workspaceRoot: '/ws', triple: 'aarch64-apple-darwin' };
    expect(sidecar.builtBinaryPath({ ...base, release: true })).toBe('/ws/target/release/masterhub-service');
    expect(sidecar.builtBinaryPath({ ...base, release: false, targetOverride: true })).toBe(
      '/ws/target/aarch64-apple-darwin/debug/masterhub-service',
    );
    expect(sidecar.builtBinaryPath({ ...base, release: false, targetDir: '/cache' })).toBe('/cache/debug/masterhub-service');
    expect(sidecar.builtBinaryPath({ ...base, release: false, targetDir: 'out' })).toBe('/ws/out/debug/masterhub-service');
    expect(
      sidecar.builtBinaryPath({ workspaceRoot: '/ws', triple: 'x86_64-pc-windows-msvc', release: true }),
    ).toBe('/ws/target/release/masterhub-service.exe');
  });

  it('parses arguments and the Tauri target env', () => {
    expect(sidecar.parseArgs([], {})).toEqual({ release: false, target: undefined });
    expect(sidecar.parseArgs(['--release', '--target', 'x'], {})).toEqual({ release: true, target: 'x' });
    expect(sidecar.parseArgs([], { TAURI_ENV_TARGET_TRIPLE: 'y' }).target).toBe('y');
    expect(() => sidecar.parseArgs(['--bogus'], {})).toThrow();
  });
});
