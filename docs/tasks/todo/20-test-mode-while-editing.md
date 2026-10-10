# 20 · Switch to Test mode while editing

**Priority:** P2 · **Effort:** S–M · **Hardware needed:** no
**Part of:** [17](17-live-vs-test-mode.md). Needs 18.

## Scope
- Setting "Switch to Test mode while editing" (on by default), with a plain description under it.
- "Editing" = a binding has unsaved changes in the inspector, or the action picker / bindings drawer is open; ends on save, revert or close. While editing, all modules are held in Test and the badge reads "Test (editing)" with the reason.
- The previous mode is restored when editing ends; if the user switched the mode manually while editing, keep their choice.

## Acceptance
- Opening a binding for edit flips Live to Test and shows why; save/close restores Live.
- Turning the setting off leaves the mode alone while editing.
- Unit tests for the editing predicate and restore rules; QA capture of "Test (editing)".
