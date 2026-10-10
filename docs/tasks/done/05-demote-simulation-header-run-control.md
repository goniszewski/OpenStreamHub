# 05 · Demote simulation; add a header Run control

**Priority:** P0 · **Effort:** M · **Hardware needed:** no
**Status:** Done · 9 October 2026 · **Branch:** master (commit f647e98, pushed)

## Why
The Simulation panel and the large Run/service panel (`DevicesPage.tsx`, `ServicePanel.tsx`) take as much space as editing, and the Run controls sit below the fold. The primary action — run my profile on my device — is hard to find.

## Scope
- Header gets one Run control with visible state: Stopped / Dry run / Executing, plus device status (already in `device-status-pill`).
- Execute keeps its confirmation and capability disclosure dialog.
- Per-control "Test" becomes a compact button + one-line result in the inspector; full simulation panel and replay move under Settings → Developer (or a collapsed section).
- Preserve all safety behaviour (dry-run-only imports, resume confirmation).

## Acceptance
- Starting/stopping a session needs no scrolling at 1485×1087.
- Existing ServicePanel/auto-start tests updated and passing.

## What was done
- **One shared run session.** All run state and logic (device selection, mode, software-mode box, blockers, confirmation, resume) moved out of `ServicePanel` into a `useRunSession` hook created once in `App.tsx`, so the header control and the Devices-page panel can never disagree. New `RunHeaderControl` renders in `header-actions` on every page: a state pill (`run-state-pill`, carrying Stopped / Starting / Dry run / Executing / Paused / Waiting / Reconnecting / Needs confirmation / Stopping / Failed — a live run is now named "Dry run" or "Executing", never just "Running") next to the one primary action: Run (disabled with its blocker as tooltip), Stop, or, for a paused run, Resume… with Stop beside it. The Execute confirmation dialog (`ExecuteDialog`) moved into the header control, so Execute keeps its capability disclosure and works from any page. "Save profile" stepped down from primary styling; Run is the primary header action.
- **Panel slimmed.** `ServicePanel` keeps the device picker, the Dry run / Execute actions toggle, the software-mode option, all session banners (including the `service-resume` Resume button), auto-start notes, dry-run-only notes, stale notice, counters and activity log; its Run/Stop buttons are gone (header only).
- **Testing in the inspector.** The workspace Simulation panel was removed. The inspector ends with a compact Test button ("Test gesture" / "Test position" / "Simulate key click"), a one-line result (`TestResult`, moved from the panel incl. `KeypadResult`), the simulated-fader-position slider for the fader module, the dial-functions preview note for the encoder, and a "Tests never execute anything" hint. Nothing user-facing was lost: per-gesture testing goes through the Gesture selector.
- **Sample test collapsed.** The Profiles page's "Test with sample input" is a collapsed `<details>` disclosure (summary row: title + "No hardware required"); contents unchanged.
- **Copy.** Onboarding "Try it without hardware" and "Connect modules" steps now describe the inspector Test button and the header Run; app README's "Run profile" bullet describes the header control.

## Verification
- `npx tsc --noEmit` clean; `npm test` 645/645 pass (was 636; ServicePanel, auto-start, mhz-import, DevicesPage and components suites updated, plus new tests: header pill states, header-only Run/Stop placement, header Resume+Stop for paused runs, fader position in the inspector, collapsed replay section); `cargo clippy --workspace --all-targets` clean.
- Screenshots (browser preview at 1485×1087; "Desktop app required" banner expected outside the native app): `qa/run-header-devices-dark.png`, `qa/run-header-profiles-dark.png`, `qa/run-header-sample-input-dark.png` (expanded), `qa/run-header-fader-dark.png`, `qa/run-header-fader-light.png`, `qa/run-header-keys-dark.png`. All six passed an independent visual review; the first sample-input capture was retaken scrolled to the expanded controls.
- Run/Stop need no scrolling: they sit in the always-visible header (verified in every capture); the browser-preview Run button is disabled with the "Running a profile needs the desktop app…" tooltip.
- Dated section appended to `apps/open-stream-hub/design-qa.md` (gitignored, as is `qa/`).

## Design decisions worth reviewing
- The scope's "Settings → Developer (or a collapsed section)" was resolved as: inspector absorbs the per-control testing (the full panel had no unique capability left), and the Profiles sample test stays on-page as a collapsed section. No Developer settings section was added.
- The header Run control appears on all pages (it runs the open profile; device and mode come from the Devices page panel). The run-state pill stays visible at narrow widths (only the device-status pill hides under 900px).

## Review follow-ups (9 October 2026)

All three review points addressed:
- **Danger styling restored.** The header Run button is `button danger` whenever it would run with actions, labelled "Run with actions on <module>" — the warning now exists before the press, not only in the pill after it.
- **Blocker visible everywhere.** A blocked Run shows its reason as a full-width strip under the header row (`RunBlockerNotice`) on every page: no module connected, imported dry-run-only, Built-in animation, unreadable settings. The Devices panel's inline blocker was removed as the duplicate, and the disabled button keeps its tooltip. The browser preview omits the strip (its own banner already explains the missing desktop app); the strip is covered by ServicePanel tests since browser captures cannot show it.
- **Device named on the button.** The header Run label names the module a start would use — "Run on 15-key" / "Run on encoder" / "Run on knobs" / "Run on faders" / "Run on rollers" (compact names in `runTargetLabel`); the tooltip keeps the full device name with serial, and the Devices panel still picks the exact device.
- **Header layout consequence (flagged for review):** with the longer labels, the header-actions row slid underneath the floating centred profile switcher (measured at 1485×1087). The switcher now sits in the header row after the brand instead of floating centred, and the app shell/header became a flex column so the blocker strip can appear without breaking the layout. Easily reverted if the centred look is preferred — but then the label and strip need another home.
- Also fixed en route: a session that reported 'stopped' while its last process state was still 'running' made the pill read "Executing"; the pill now treats a reported-stopped session as Stopped (`phaseLabel`/`phaseClass`).

## Follow-up verification
- `npx tsc --noEmit` clean; `npm test` 648/648 pass (new tests: danger styling and full Execute label, target naming on the button, blocker strip present on a blocked native run and absent for an active run and in the browser preview, no `service-blocker` duplicate).
- All six `qa/run-header-*.png` re-captured and passed an independent visual review (no overlaps in either theme after the header restructure). Dated follow-up section appended to `apps/open-stream-hub/design-qa.md`; README "Run profile" bullet updated.
