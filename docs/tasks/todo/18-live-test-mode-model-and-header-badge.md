# 18 · Live / Test mode: global mode model and header badge

**Priority:** P1 · **Effort:** M · **Hardware needed:** yes for the final press check
**Part of:** [17](17-live-vs-test-mode.md) (spec and decisions). Build first; 19 and 20 depend on it.

## Scope
- One global mode (Live / Test) held by the app and applied to every running module, including runs the app starts itself. Replaces the per-run `execute` choice on the Run panel (`ServicePanel.tsx`) and the "Run actions automatically" setting (migration per 17, decision 6).
- Header badge showing the one true state: "Live", "Test", "Test (imported profile)"; a switch to change it. Remove the "Dry run (default) / Execute actions" toggle and make banner, header button and Run panel wording agree.
- Switching applies to running sessions without a restart (backend session mode change, not stop/start).
- Live default behind the one-time "what this profile will do" confirmation (17, decision 1); imported / unreviewed profiles stay Test; macOS-only execution unchanged.
- Test feedback: pressed tile flashes and the log reads "Would run: …" (17, decision 4).

## Acceptance
- The badge always matches what a press does; Live executes, Test only logs, switching needs no restart.
- Imported profile cannot enter Live until allowed; tests cover it. Migration tests for users who had auto-execute on/off.
- Onboarding and Settings copy updated; QA captures (Live, Test, imported-locked); on-device check in `docs/masterhub-hardware-results.md`.
