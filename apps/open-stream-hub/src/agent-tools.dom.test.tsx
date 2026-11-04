// @vitest-environment jsdom
import { act } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AgentToolsPanel } from './AgentToolsPanel';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { AgentToolStatus } from './agent-tools-model';
import type { Invoke } from './profile-session';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; vi.useRealTimers(); });

const status = (patch: Partial<AgentToolStatus> = {}): AgentToolStatus => ({
  osh_path: '/Applications/OpenStreamHub.app/Contents/MacOS/osh', translocated: false, link_path: '/Users/a/.local/bin/osh', linked: false, on_path: false,
  claude_config: '{\n  "hooks": {}\n}', codex_config: '[[hooks.Stop]]', ...patch,
});

// An invoke that answers each command from `answers` and records the calls.
function fakeInvoke(answers: Partial<Record<string, () => AgentToolStatus>>) {
  const calls: string[] = [];
  const invoke = (async (command: string) => {
    calls.push(command);
    const answer = answers[command];
    if (!answer) throw new Error(`unexpected ${command}`);
    return answer();
  }) as Invoke;
  return { invoke, calls };
}

// Lets the pending invoke promises settle inside act().
const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

const buttonNamed = (view: Rendered, text: string) => {
  const found = view.$$<HTMLButtonElement>('button').find(button => button.textContent === text);
  if (!found) throw new Error(`no button ${text}`);
  return found;
};

describe('the AI agents panel in the DOM', () => {
  it('shows only a note in the browser preview', () => {
    view = render(<AgentToolsPanel desktop={false} />);
    expect(view.$('h2').textContent).toBe('AI agents (Claude Code, Codex)');
    expect(view.container.textContent).toContain('Set up in the desktop app.');
    expect(view.$$('button')).toEqual([]);
  });

  it('links osh, shows the PATH line, and removes the link', async () => {
    let linked = false;
    const { invoke, calls } = fakeInvoke({
      agent_tool_status: () => status({ linked }),
      link_agent_tool: () => { linked = true; return status({ linked }); },
      unlink_agent_tool: () => { linked = false; return status({ linked }); },
    });
    view = render(<AgentToolsPanel desktop invoke={invoke} copy={async () => true} />);
    await settle();
    expect(calls).toEqual(['agent_tool_status']);
    expect(view.container.textContent).toContain('/Applications/OpenStreamHub.app/Contents/MacOS/osh');
    expect(view.container.textContent).toContain('Not linked.');
    expect(view.container.textContent).toContain('export PATH="$HOME/.local/bin:$PATH"');
    expect(view.$$('pre').map(pre => pre.textContent)).toEqual(['{\n  "hooks": {}\n}', '[[hooks.Stop]]']);
    expect(view.container.textContent).toContain('~/.claude/settings.json');
    expect(view.container.textContent).toContain('~/.codex/config.toml');
    expect(view.container.textContent).toContain('run /hooks in Codex');

    view.click(buttonNamed(view, 'Make osh available in the terminal'));
    await settle();
    expect(calls.at(-1)).toBe('link_agent_tool');
    expect(view.container.textContent).toContain('osh is linked at /Users/a/.local/bin/osh.');
    expect(view.$('.agent-tools-result').textContent).toBe('Linked. Open a new terminal window to use osh.');

    view.click(buttonNamed(view, 'Remove link'));
    await settle();
    expect(calls.at(-1)).toBe('unlink_agent_tool');
    expect(view.$('.agent-tools-result').textContent).toBe('Link removed.');
    expect(buttonNamed(view, 'Make osh available in the terminal')).toBeTruthy();
  });

  it('hides the PATH line when ~/.local/bin is on PATH, and reports a refused link', async () => {
    const { invoke } = fakeInvoke({ agent_tool_status: () => status({ on_path: true }), link_agent_tool: () => { throw new Error('/Users/a/.local/bin/osh exists and is not a link, so it was left alone'); } });
    view = render(<AgentToolsPanel desktop invoke={invoke} copy={async () => true} />);
    await settle();
    expect(view.container.textContent).not.toContain('export PATH');
    view.click(buttonNamed(view, 'Make osh available in the terminal'));
    await settle();
    expect(view.$('[role="alert"]').textContent).toContain('is not a link, so it was left alone');
    expect(view.container.querySelector('.agent-tools-result')).toBeNull();
  });

  it('says so when the bundled osh is missing, with no link button or configuration', async () => {
    const { invoke } = fakeInvoke({ agent_tool_status: () => status({ osh_path: null, claude_config: '', codex_config: '' }) });
    view = render(<AgentToolsPanel desktop invoke={invoke} />);
    await settle();
    expect(view.container.textContent).toContain('The bundled osh was not found');
    expect(view.$$('pre')).toEqual([]);
    expect(view.$$('button')).toEqual([]);
  });

  it('asks to move a translocated app to Applications, with no link button or configuration', async () => {
    const { invoke } = fakeInvoke({ agent_tool_status: () => status({ osh_path: null, translocated: true, claude_config: '', codex_config: '' }) });
    view = render(<AgentToolsPanel desktop invoke={invoke} />);
    await settle();
    expect(view.container.textContent).toContain('Move OpenStreamHub to Applications and open it from there.');
    expect(view.container.textContent).not.toContain('The bundled osh was not found');
    expect(view.$$('pre')).toEqual([]);
    expect(view.$$('button')).toEqual([]);
  });

  it('copies a configuration and shows Copied for a moment, or says copying failed', async () => {
    vi.useFakeTimers();
    const copied: string[] = [];
    let works = true;
    const { invoke } = fakeInvoke({ agent_tool_status: () => status() });
    view = render(<AgentToolsPanel desktop invoke={invoke} copy={async text => { copied.push(text); return works; }} />);
    await settle();
    const claude = () => view!.$<HTMLButtonElement>('[aria-label="Copy the Claude Code configuration"]');
    view.click(claude());
    await settle();
    expect(copied).toEqual(['{\n  "hooks": {}\n}']);
    expect(claude().textContent).toBe('Copied');
    act(() => { vi.advanceTimersByTime(2000); });
    expect(claude().textContent).toBe('Copy');

    view.click(view.$('[aria-label="Copy the Codex configuration"]'));
    view.click(view.$('[aria-label="Copy the PATH line"]'));
    await settle();
    expect(copied.slice(1)).toEqual(['[[hooks.Stop]]', 'export PATH="$HOME/.local/bin:$PATH"']);

    works = false;
    view.click(claude());
    await settle();
    expect(view.$('[role="alert"]').textContent).toBe('Could not copy. Select the text and copy it yourself.');
  });
});
