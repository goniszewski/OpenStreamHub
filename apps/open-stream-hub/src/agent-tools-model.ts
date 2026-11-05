// The AI agents panel in Settings: what the app reports about its bundled `osh`, and the small decisions the panel
// makes from it. The app links `osh` into ~/.local/bin on request; the agents' own settings files are never edited.
import type { Invoke } from './profile-session';

export interface AgentToolStatus {
  osh_path: string | null;
  // macOS runs the app from an App Translocation copy, whose path changes at every launch: nothing is offered.
  translocated: boolean;
  link_path: string;
  linked: boolean;
  // Only the app's own PATH is checked: a terminal's login shell may add ~/.local/bin, so false is not proof.
  on_path: boolean;
  claude_config: string;
  codex_config: string;
}

export type AgentToolCommand = 'agent_tool_status' | 'link_agent_tool' | 'unlink_agent_tool';

export type HookState = 'configured' | 'not_configured' | 'unreadable';

// What `agent_signals` reports. The hook states come from reading the agents' settings files (never writing them); the
// answer count covers prompts the deck answered since the app started and is not stored.
export interface AgentSignalsReply {
  claude: HookState;
  codex: HookState;
  answered: number;
  last_answer_ms: number | null;
}

export const readAgentSignals = (invoke: Invoke) => invoke<AgentSignalsReply>('agent_signals');

// The checklist's done flags from the app's report; nothing is done before it has reported.
export const signalsSeen = (reply: AgentSignalsReply | null): AgentSignals => ({
  claude: reply?.claude === 'configured',
  codex: reply?.codex === 'configured',
  answered: (reply?.answered ?? 0) > 0,
});

export const runAgentTool = (invoke: Invoke, command: AgentToolCommand) => invoke<AgentToolStatus>(command);

// The line that puts ~/.local/bin on a zsh or bash PATH. Fixed text: no path from the app goes into it.
export const PATH_EXPORT = 'export PATH="$HOME/.local/bin:$PATH"';

export const agentTargets = [
  { agent: 'claude', file: '~/.claude/settings.json', config: (status: AgentToolStatus) => status.claude_config },
  { agent: 'codex', file: '~/.codex/config.toml', config: (status: AgentToolStatus) => status.codex_config },
] as const;

export type OshState = 'translocated' | 'missing' | 'linked' | 'unlinked';

export const oshState = (status: AgentToolStatus): OshState => status.translocated ? 'translocated' : !status.osh_path ? 'missing' : status.linked ? 'linked' : 'unlinked';

// The PATH line is offered while osh can be linked and the app could not see ~/.local/bin on its PATH.
export const showPathHint = (status: AgentToolStatus) => !!status.osh_path && !status.on_path;

export type ChecklistStep = 'osh' | 'claude' | 'codex' | 'answered';

// What the app has actually seen (`signalsSeen`): hooks found in an agent's settings file, a prompt answered from the
// deck this session. A step with no such signal is not done; nothing is guessed.
export type AgentSignals = Partial<Record<Exclude<ChecklistStep, 'osh'>, boolean>>;

// osh counts as available when the bundled one was found at a stable path: the hooks call it by its full path, so
// linking it into ~/.local/bin is optional and not part of this step.
export const agentChecklist = (status: AgentToolStatus, seen: AgentSignals = {}) => {
  const steps = [
    { step: 'osh' as const, done: oshState(status) === 'linked' || oshState(status) === 'unlinked' },
    { step: 'claude' as const, done: !!seen.claude },
    { step: 'codex' as const, done: !!seen.codex },
    { step: 'answered' as const, done: !!seen.answered },
  ];
  // Ready needs only one agent: someone who uses Claude Code alone would otherwise never leave the checklist.
  const [osh, claude, codex, answered] = steps;
  return { steps, ready: osh.done && (claude.done || codex.done) && answered.done };
};

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
