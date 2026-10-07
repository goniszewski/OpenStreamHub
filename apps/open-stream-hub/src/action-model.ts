import type { Action, MediaCommand, ProfileDirection, StepAction, SystemCommand } from './contracts';

// Mirrors NAMED_KEYS in crates/cooltek-sdk-service/src/profile.rs. The Rust
// service holds the authoritative allow-list and rejects anything else.
export const namedKeys: ReadonlyArray<readonly [string, string]> = [
  ['return', 'Return'], ['tab', 'Tab'], ['space', 'Space'], ['delete', 'Delete'], ['escape', 'Escape'],
  ['forward_delete', 'Forward Delete'], ['home', 'Home'], ['end', 'End'], ['page_up', 'Page Up'], ['page_down', 'Page Down'],
  ['arrow_left', 'Left Arrow'], ['arrow_right', 'Right Arrow'], ['arrow_down', 'Down Arrow'], ['arrow_up', 'Up Arrow'],
  ...Array.from({ length: 20 }, (_, index) => [`f${index + 1}`, `F${index + 1}`] as const),
];

export function keyLabel(key: string) {
  return namedKeys.find(([name]) => name === key)?.[1] ?? (key === ' ' ? 'Space' : key.toUpperCase());
}

export const mediaLabels: Record<MediaCommand, string> = {
  play_pause: 'Play/pause media', next_track: 'Next track', previous_track: 'Previous track', stop: 'Stop media',
};

export const systemLabels: Record<SystemCommand, string> = {
  undo: 'Undo', redo: 'Redo', screenshot: 'Screenshot to clipboard',
};

// Mirrors ProfileDirection in crates/cooltek-sdk-service/src/profile.rs (a closed set).
export const profileDirectionLabels: Record<ProfileDirection, string> = {
  next: 'Next profile', previous: 'Previous profile',
};

// Limits enforced by the Rust service (Action::MAX_* constants).
export const maxUrlBytes = 2048;
export const maxTextBytes = 2048;
export const multiActionSteps = { minimum: 2, maximum: 8 } as const;

export type StepActionType = StepAction['type'];

export const stepActionTypes: ReadonlyArray<readonly [StepActionType, string]> = [
  ['volume', 'Change output volume'], ['set_volume', 'Set output volume'], ['toggle_mute', 'Toggle output mute'],
  ['shortcut', 'Keyboard shortcut'], ['launch_app', 'Open application'], ['open_url', 'Open URL'],
  ['text', 'Paste text'], ['media', 'Media control'], ['system', 'System action'],
];

export function defaultStepAction(type: StepActionType): StepAction {
  switch (type) {
    case 'volume': return { type, delta: 5 };
    case 'set_volume': return { type, percent: 50 };
    case 'toggle_mute': return { type };
    case 'shortcut': return { type, key: 'c', modifiers: ['command'] };
    case 'launch_app': return { type, bundle_id: 'com.apple.finder' };
    case 'open_url': return { type, url: 'https://example.com' };
    case 'text': return { type, text: 'Hello', press_enter: false };
    case 'media': return { type, command: 'play_pause' };
    case 'system': return { type, command: 'undo' };
  }
}

export function defaultProfileAction(): Action {
  return { type: 'profile', direction: 'next' };
}

export function defaultMultiAction(): Action {
  return { type: 'multi_action', actions: [{ type: 'toggle_mute' }, defaultStepAction('shortcut')] };
}

const utf8 = new TextEncoder();

// Inline hints only; the Rust validator is authoritative and stricter.
export function urlIssue(url: string): string | null {
  if (!/^https?:\/\/[^/?#]/.test(url)) return 'Enter an http:// or https:// address with a host.';
  if (utf8.encode(url).length > maxUrlBytes) return `Use at most ${maxUrlBytes} bytes.`;
  if (/[^A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=%]/.test(url)) return 'Percent-encode spaces, quotes and non-ASCII characters.';
  if (/^https?:\/\/[^/?#]*@/.test(url)) return 'Remove the user name or password from the address.';
  return null;
}

// The scheme and authority of an address (scheme://host[:port]), never its path, query or fragment, which can carry
// tokens. User information is dropped. Used for the activity log and the Execute confirmation.
export function urlOrigin(url: string): string {
  const match = /^([A-Za-z][A-Za-z0-9+.-]*:)\/\/(?:[^/?#]*@)?([^/?#]*)/.exec(url);
  return match ? `${match[1].toLowerCase()}//${match[2].toLowerCase()}` : 'an unrecognized address';
}

// Mirrors is_invisible_text_character in crates/cooltek-sdk-service/src/profile.rs: format (Cf) characters such as
// zero-width spaces, bidi controls, soft hyphens and the BOM, the line/paragraph separators, and the whole
// U+2060-206F block (U+2065 is unassigned). They are invisible in an editor, so the pasted text could differ from
// what the user reviewed. Emoji ZWJ sequences (U+200D) are rejected too, as in Rust.
export const invisibleTextCharacter = /[\p{Cf}\p{Zl}\p{Zp}\u{2060}-\u{206F}]/u;

export function textIssue(text: string): string | null {
  if (!text) return 'Enter the text to paste.';
  if (utf8.encode(text).length > maxTextBytes) return `Use at most ${maxTextBytes} bytes.`;
  if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(text)) return 'Only newlines and tabs are allowed as control characters.';
  if (invisibleTextCharacter.test(text)) return 'Remove invisible characters (zero-width spaces, direction marks, line or paragraph separators).';
  return null;
}
