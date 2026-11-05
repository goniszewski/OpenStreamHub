// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { PATH_EXPORT, agentChecklist, agentTargets, copyText, oshState, readAgentSignals, runAgentTool, showPathHint, signalsSeen } from './agent-tools-model';
import type { AgentToolStatus } from './agent-tools-model';
import type { Invoke } from './profile-session';

const status = (patch: Partial<AgentToolStatus> = {}): AgentToolStatus => ({
  osh_path: '/Applications/OpenStreamHub.app/Contents/MacOS/osh', translocated: false, link_path: '/Users/a/.local/bin/osh', linked: false, on_path: false,
  claude_config: '{"hooks":{}}', codex_config: '[[hooks.Stop]]', ...patch,
});

describe('the AI agents panel model', () => {
  it('names the state of osh', () => {
    expect(oshState(status({ osh_path: null, linked: true }))).toBe('missing');
    expect(oshState(status())).toBe('unlinked');
    expect(oshState(status({ linked: true }))).toBe('linked');
    expect(oshState(status({ osh_path: null, translocated: true }))).toBe('translocated');
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

describe('agentChecklist', () => {
  it('marks osh done once the bundled one is found at a stable path, linked or not', () => {
    expect(agentChecklist(status()).steps[0]).toEqual({ step: 'osh', done: true });
    expect(agentChecklist(status({ linked: true })).steps[0].done).toBe(true);
    expect(agentChecklist(status({ osh_path: null })).steps[0].done).toBe(false);
    expect(agentChecklist(status({ translocated: true })).steps[0].done).toBe(false);
  });
  it('leaves the steps the app cannot observe not done, so it is never ready on its own', () => {
    const { steps, ready } = agentChecklist(status());
    expect(steps.map(step => step.done)).toEqual([true, false, false, false]);
    expect(ready).toBe(false);
  });
  it('is ready only when every step is seen', () => {
    expect(agentChecklist(status(), { claude: true, codex: true }).ready).toBe(false);
    expect(agentChecklist(status(), { claude: true, codex: true, answered: true }).ready).toBe(true);
  });
});

describe('signalsSeen', () => {
  it('is done only for configured hooks and a positive answer count', () => {
    expect(signalsSeen(null)).toEqual({ claude: false, codex: false, answered: false });
    expect(signalsSeen({ claude: 'configured', codex: 'unreadable', answered: 0, last_answer_ms: null })).toEqual({ claude: true, codex: false, answered: false });
    expect(signalsSeen({ claude: 'not_configured', codex: 'configured', answered: 3, last_answer_ms: 9 })).toEqual({ claude: false, codex: true, answered: true });
  });
  it('asks the app for agent_signals', async () => {
    const invoke = vi.fn(async () => ({ claude: 'configured' })) as unknown as Invoke;
    await readAgentSignals(invoke);
    expect(invoke).toHaveBeenCalledWith('agent_signals');
  });
});
