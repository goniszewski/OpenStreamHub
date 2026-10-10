import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Build provenance for the About page. Outside a git checkout (a source archive) the git fields are null.
const git = (args: string): string | null => {
  try {
    return execSync(`git ${args}`, { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
  } catch {
    return null;
  }
};
const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')) as { version: string };
const commit = git('rev-parse --short HEAD');
const buildInfo = {
  version: pkg.version,
  commit,
  commitDate: git('log -1 --format=%as HEAD'),
  dirty: commit === null ? null : git('status --porcelain') !== '',
  builtAt: new Date().toISOString().slice(0, 10),
};

export default defineConfig({
  plugins: [react()],
  define: { __BUILD_INFO__: JSON.stringify(buildInfo) },
  clearScreen: false,
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
  build: { target: 'safari15' },
  test: { environment: 'node', include: ['src/**/*.test.{ts,tsx}', 'scripts/**/*.test.ts'] },
});
