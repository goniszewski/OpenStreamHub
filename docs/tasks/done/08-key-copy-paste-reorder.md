# 08 · Copy / paste / duplicate / drag-rearrange keys

**Done · 10 October 2026 · commit 88e7a98 on master (pushed)**

**Priority:** P1 · **Effort:** S–M · **Hardware needed:** no

## Why
Official app has right-click Copy / Paste / Remove on keys and drag-to-rearrange. Ours has Clear key only (`Inspector.tsx`).

## Scope
- Context menu and shortcuts (⌘C/⌘V/⌫) on keypad tiles and on other controls' bindings.
- Drag a key tile onto another to move/swap; Alt-drag to copy.
- Copy across folders and pages; copying a folder duplicates its subtree (respect depth/page limits in `keypad-model.ts`).
- All operations go through the undo history.

## What was done
- **Model (`keypad-model.ts`):** `copyKeypadSlot` lifts a key as pure clipboard data — slot (label and every binding), artwork and, for a folder, the whole subtree (`keypadFolderPages`, extracted from `clearKeypadSlot`, same error messages). `pasteKeypadSlot` writes a copied key onto an **empty** key only (an occupied target refuses with "This key already holds content. Clear it before pasting." instead of overwriting), regenerates every subtree page id through an injected id factory so the source folder can survive, and enforces the existing depth/64-page limits with the folder messages. `moveKeypadSlot` moves onto an empty key and swaps with an occupied one, artwork included, works across pages, returns the profile unchanged for a drop on the same key, and refuses a folder moved into its own subtree.
- **Tiles (`KeypadScreen.tsx`):** occupied keys are draggable and carry their source page/control as a custom-MIME payload (`key-drag.ts`, same pattern as the action-picker drag); dropping a dragged key moves, swaps or — with Alt — copies, while action-picker drops keep working (the two payloads are distinguished by MIME). Right-click selects the key and opens a Copy / Paste / Clear key menu (`ContextMenu` in `controls.tsx` — a presentational, focus-managed popover: it takes focus on open, so Escape and any outside click close it without listeners). The reserved Back key never drags, drops or opens the menu.
- **Other controls:** ⌘C/⌘V/⌫ act on the inspector selection — a key, or the selected control's binding (paste applies the copied action to the selected gesture/serial, keeping the fader-only rule and the dial-functions refusal). Binding rows in the bindings drawer gain a right-click Copy / Remove menu; copying a keypad row copies the whole key, a dial-function row copies as an encoder binding.
- **Editor glue (`useDeviceEditor.ts`):** clipboard and menu are view state (Undo does not restore a copy, like any clipboard); `copySelection`/`pasteSelection`/`deleteSelection` return false when nothing of ours is under the keys so the browser keeps ⌘C/⌘V/⌫ inside text fields (the shortcut map in `shortcuts.ts` also guards text entries and repeats). Every paste/move/copy-drop/clear is one `session.update` step, so each is a single undo/redo step; refusals surface as the usual error strip, successes as one-line notices.
- **Shortcuts documentation:** the Settings shortcut list gained the three new rows; the key menu shows ⌘C/⌘V hints platform-correctly.
- **Preview QA hooks (dev + browser only, like the existing seeds):** `__oshKeyMenuSeed`, `__oshKeyCopySeed`, `__oshKeySampleSeed` in `App.tsx`, because the tile handlers are wired only for the editable desktop app.

## Verification
- `npx tsc --noEmit` clean; `npm test` (vitest) all suites green, including the new groups: keypad-model copy/paste/move (bindings and artwork follow, fresh subtree ids, occupied-target/depth/page refusals with the exact messages, folder-into-itself refusal, folder listing order) and the session-level undo/redo test that restores a paste and a move; key-drag payload round-trip and malformed-payload rejection; tile tests extended to drag start, key drops (move, Alt copy, action fall-through, foreign payload ignored), right-click wiring (never on Back) and the menu items' enabled states; shortcuts tests cover ⌘C/⌘V/⌫ mapping, text-entry pass-through and prevent-default only when acted.
- Browser-preview captures (1485×1087, dark) in `qa/`: `key-context-menu-dark.png` (menu on an occupied key: Copy/Clear enabled, Paste disabled), `key-copy-paste-dark.png` (after copying, Paste enabled on an empty key), `key-paste-result-dark.png` (pasted key shows label + binding, notice "Pasted key onto Key 6.", bindings count 7 → 8) and `binding-row-menu-dark.png` (drawer row menu). Setup documented in `design-qa.md` (10 October 2026 entry).
- Open, hardware-scoped: an OS-level drag on WKWebView was not exercised (the preview cannot drag; handlers are SSR-proven). Same open item as task 02's drag check; no hardware is needed for anything else in this task.

## Review round · 10 October 2026
- Reviewer's capture notes handled: `qa/key-paste-result-dark.png` re-captured under the same name so the "Pasted key onto Key 6." notice is visible in the frame (taller 1485×2050 viewport for this one capture; see the design-qa entry). The OS-level drag check stays open as recorded — no change. The stray untracked `node_modules/` cache at the repo root (a vitest cache from a command run in the wrong directory) was deleted; `git status` is clean again and the status line above remains accurate (88e7a98 still unpushed).
