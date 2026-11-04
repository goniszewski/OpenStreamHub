// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { PATH_EXPORT, agentTargets, copyText, oshState, runAgentTool, showPathHint } from './agent-tools-model';
import type { AgentToolStatus } from './agent-tools-model';
import type { Invoke } from './profile-session';

const status = (patch: Partial<AgentToolStatus> = {}): AgentToolStatus => ({
  osh_path: '/Applications/OpenStreamHub.app/Contents/MacOS/osh', link_path: '/Users/a/.local/bin/osh', linked: false, on_path: false,
  claude_config: '{"hooks":{}}', codex_config: '[[hooks.Stop]]', ...patch,
});

describe('the AI agents panel model', () => {
  it('names the state of osh', () => {
    expect(oshState(status({ osh_path: null, linked: true }))).toBe('missing');
    expect(oshState(status())).toBe('unlinked');
    expect(oshState(status({ linked: true }))).toBe('linked');
  });

  it('offers the PATH line only while osh exists and ~/.local/bin is not seen on PATH', () => {
    expect(showPathHint(status())).toBe(true);
    expect(showPathHint(status({ on_path: true }))).toBe(false);
    expect(showPathHint(status({ osh_path: null }))).toBe(false);
    expect(PATH_EXPORT).toBe('export PATH="$HOME/.local/bin:$PATH"');
  });

  it('pairs each agent with its file and configuration', () => {
    expect(agentTargets.map(target => [target.agent, target.file, target.config(status())])).toEqual([
      ['claude', '~/.claude/settings.json', '{"hooks":{}}'],
      ['codex', '~/.codex/config.toml', '[[hooks.Stop]]'],
    ]);
  });

  it('calls the named command with no arguments', async () => {
    const invoke = vi.fn(async () => status()) as unknown as Invoke;
    await expect(runAgentTool(invoke, 'link_agent_tool')).resolves.toEqual(status());
    expect(invoke).toHaveBeenCalledWith('link_agent_tool');
  });

  it('copies through the clipboard, else through the copy command', async () => {
    const writeText = vi.fn(async () => {});
    expect(await copyText('a', { writeText }, document)).toBe(true);
    expect(writeText).toHaveBeenCalledWith('a');

    const execCommand = vi.fn(() => true);
    Object.defineProperty(document, 'execCommand', { value: execCommand, configurable: true });
    const refused = { writeText: vi.fn(async () => { throw new Error('denied'); }) };
    expect(await copyText('b', refused, document)).toBe(true);
    expect(await copyText('c', undefined, document)).toBe(true);
    expect(execCommand).toHaveBeenCalledWith('copy');
    expect(document.querySelector('textarea')).toBeNull();

    execCommand.mockReturnValue(false);
    expect(await copyText('d', undefined, document)).toBe(false);
    expect(await copyText('e', undefined, undefined)).toBe(false);
  });
});
