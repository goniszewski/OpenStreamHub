# 06 · Merge Devices and Profiles into one workspace

**Status:** Done · 9 October 2026 · commits 9ab8e74, 680ba27 and 2f2facc on master (unpushed)

**Priority:** P1 · **Effort:** L · **Hardware needed:** no

## Why
Editing a binding is on Devices; seeing all bindings, validating, import/export, focus rules and dial functions are on Profiles (`ProfilesPage.tsx`). Users bounce between pages. The official app is one screen.

## Scope
- Single workspace: module canvas (centre), collapsible bindings list drawer, inspector (right).
- Profile manager (create/duplicate/delete/import/export, unreadable profiles) becomes a popover or sheet from the header profile switcher.
- Move focus rules next to the profile ("Switch automatically for…", see task 13); move dial functions into the dial inspector (see task 10).
- Navigation reduces to: Workspace · Display artwork · Settings · Compatibility.
- Keep keyboard shortcuts (`shortcuts.ts`) working or remap.

## Acceptance
- All Profiles-page capabilities reachable without a separate page.
- No regression in draft/restore/unsaved-guard behaviour; tests updated.

## Depends on
01, 05 (reduces what has to be moved).

## What was done
- **One workspace page.** `Page` is now `workspace | artwork | compatibility | settings`; the sidebar lists Workspace, Display artwork and Compatibility, with Settings at the bottom. The workspace keeps the three columns (module catalog, canvas with the run panel, inspector) and adds, under the canvas, the collapsible **bindings drawer** (`BindingsDrawer` in `DevicesPage.tsx` — closed it shows the binding count and, in an accent pill, the number of binding warnings; open it is the full searchable `BindingsPanel`) and the **Test with sample input** panel (`SampleTestPanel`). Profile files can be dropped anywhere on the workspace to import them.
- **Profile manager sheet from the header.** `ProfilesPage.tsx` became `ProfileManager.tsx`: a sheet (`ProfileManagerSheet`) opens from a new "Manage profiles" button next to the header profile switcher (Escape, the close button or an outside click closes it; the workspace stays visible). It holds, in one scrollable panel: the open profile's name with Saved/Unsaved, Validate changes, Reload saved, Earlier versions (`VersionHistory`), the imported-profile dry-run banner with the allow action, the damaged-import-list recovery, the saved-profile list (switch/delete, dry-run badges), unreadable profiles, import/export (`ProfileTransfer`) and the focused-app rules (`FocusRulesPanel`, per task 13's "next to the profile"). The import report and allow-execution dialogs moved to App root so a modal never nests inside the sheet.
- **Dial functions moved into the dial inspector** (the placement task 10 builds tabs on): for the encoder, the inspector always hosts `DialFunctionsPanel` — with functions off, below the flat editor with its "Add dial functions" affordance; with functions on, as the only editor (flat encoder bindings are illegal then). The old "Edit them under Profiles · Dial functions" notes (inspector note, drop refusal) are rewritten to point at the editor in place, and a binding-list "Edit" on a dial-function binding now selects the encoder first and focuses the function's name field. The flat-encoder-bindings warning is shown once: while the dial editor is open, the generic "Binding warnings" card drops it (the editor's own block names the bindings and carries the Move action); the bindings drawer still lists it.
- **Navigation plumbing.** `selectBinding`/`selectKeypadBinding` no longer switch pages; `useDeviceEditor` lost its `setPage` parameter; the NoticeRegion "Review" action now goes to the workspace where the run panel lives. ⌘1–4 remapped to Workspace/Display artwork/Compatibility/Settings; ⌘F opens the bindings drawer and focuses its search (elsewhere the browser keeps ⌘F). Onboarding, run-blocker strips, Execute confirmation copy, display-setting notes, the compatibility table and Settings copy no longer name the removed pages: instructions name the **Manage profiles** button (next to the profile switcher), prose calls it the profile manager, and dial-function pointers name the editor in place. `docs/masterhub-focus-rules.md`, `docs/importing-masterhub-profiles.md`, `docs/masterhub-first-hardware-session.md`, `docs/masterhub-dial-functions.md` and `docs/masterhub-implementation-status.md` were synced too, as was the README.
- Draft/restore/unsaved-guard behaviour untouched: the session, its guards, autosave and the recovery prompts were not modified.

## Verification
- `npx tsc --noEmit` clean; `npm test`: 47 files, **748 tests pass** (was 719 + 27 service/app suites unchanged). Updated: `components.test.tsx` (sheet sections, drawer collapsed/open, warning pill, sample panel), `mhz-import.test.tsx` (flagged-profile banner, damaged-list recovery, import report re-open against the sheet), `DevicesPage.test.tsx` (new encoder-inspector-with-dial-functions cases), `shortcuts.test.ts` (new page map), `DialFunctionsPanel.test.tsx` (import path). No regression in profile-session/draft/restore suites.
- `cargo fmt --check` and `cargo clippy --workspace --all-targets` clean in both workspaces (no Rust changes).
- QA captures in `qa/`: `workspace-dark.png`, `bindings-drawer-dark.png`, `bindings-drawer-closed-dark.png` (the collapsed row with its count and the accent warning pill), `profile-manager-dark.png`, `dial-inspector-dark.png` (re-captured after the warning dedup: the inspector shows the dial editor's block with the Move action and no repeated "Binding warnings" card; browser preview, dark, 1485×1087; devtools lifts only — see `design-qa.md`). Captures named after the removed pages are left as history.
