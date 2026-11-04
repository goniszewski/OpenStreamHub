// The AI agents panel in Settings: what the app reports about its bundled `osh`, and the small decisions the panel
// makes from it. The app links `osh` into ~/.local/bin on request; the agents' own settings files are never edited.
import type { Invoke } from './profile-session';

export interface AgentToolStatus {
  osh_path: string | null;
  link_path: string;
  linked: boolean;
  // Only the app's own PATH is checked: a terminal's login shell may add ~/.local/bin, so false is not proof.
  on_path: boolean;
  claude_config: string;
  codex_config: string;
}

export type AgentToolCommand = 'agent_tool_status' | 'link_agent_tool' | 'unlink_agent_tool';

export const runAgentTool = (invoke: Invoke, command: AgentToolCommand) => invoke<AgentToolStatus>(command);

// The line that puts ~/.local/bin on a zsh or bash PATH. Fixed text: no path from the app goes into it.
export const PATH_EXPORT = 'export PATH="$HOME/.local/bin:$PATH"';

export const agentTargets = [
  { agent: 'claude', file: '~/.claude/settings.json', config: (status: AgentToolStatus) => status.claude_config },
  { agent: 'codex', file: '~/.codex/config.toml', config: (status: AgentToolStatus) => status.codex_config },
] as const;

export type OshState = 'missing' | 'linked' | 'unlinked';

export const oshState = (status: AgentToolStatus): OshState => !status.osh_path ? 'missing' : status.linked ? 'linked' : 'unlinked';

// The PATH line is offered while osh can be linked and the app could not see ~/.local/bin on its PATH.
export const showPathHint = (status: AgentToolStatus) => !!status.osh_path && !status.on_path;

export interface ClipboardLike { writeText(text: string): Promise<void> }

// Copies `text`: the async clipboard when there is one, else a hidden text area and the copy command. False when
// neither works, so the panel can say so; the text stays selectable in the panel either way.
export async function copyText(text: string, clipboard: ClipboardLike | undefined = globalThis.navigator?.clipboard, doc: Document | undefined = globalThis.document): Promise<boolean> {
  if (clipboard) {
    try { await clipboard.writeText(text); return true; } catch { /* falls back below */ }
  }
  if (!doc?.body) return false;
  const area = doc.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.opacity = '0';
  doc.body.appendChild(area);
  area.select();
  try { return typeof doc.execCommand === 'function' && doc.execCommand('copy'); } catch { return false; } finally { area.remove(); }
}
