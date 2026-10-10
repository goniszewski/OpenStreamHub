# 13 · Surface focused-app rules and undo/redo as differentiators

**Priority:** P2 · **Effort:** S–M · **Hardware needed:** no

## Why
The official app has neither per-app profile switching nor editor undo/redo; both exist here but hide in places (focus rules at the bottom of Profiles, undo only as header icons).

## Scope
- Per-profile row in the profile menu/inspector: "Switch automatically when [App] is in front" with an app picker (running apps + browse).
- Show the active rule/auto-switch state in the header status.
- Make Undo/Redo discoverable: Edit-history popover listing recent changes; keep ⌘Z/⇧⌘Z.
- Short explanatory tooltip on first use only.

## Acceptance
- Focus rules can be created entirely from the workspace; existing `FocusRulesPanel` behaviour covered by tests.
