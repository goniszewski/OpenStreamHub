# 01 · Strip placeholders and developer jargon from the UI

**Priority:** P0 · **Effort:** S · **Hardware needed:** no
**Status:** Done · **Completed:** 9 October 2026 · **Branch:** master (commits 30a980a and README fix)

## Why
The UI feels unfinished and developer-facing: a disabled "Flows" nav item, "Run flow / Run script — Planned" in every inspector, "Planned" / "Device test pending" badges on module cards, and copy such as "The Rust service applies exact-serial precedence during preview and replay".

## Scope
- Remove the disabled Flows nav item (`App.tsx`) and the `planned-actions` block in `Inspector.tsx` until those features exist.
- Replace "Planned"/"Device test pending" card sublabels with a neutral state (connected / not connected); move support detail to Compatibility.
- Rename jargon: Draft → "Unsaved changes", "Replay capture" → "Test with sample input", "Module control" → "Let OpenStreamHub drive the display", "Device selector" → under an Advanced disclosure.
- Collapse multi-paragraph notes (timer, OBS, profile action in `ActionFields.tsx`) to one-line hints, with details behind a "Learn more" disclosure.
- Update affected tests and onboarding copy.

## Acceptance
- No "Planned" label or disabled nav item visible in the default UI.
- Serial scoping is hidden under Advanced; default flow never mentions "Rust", "service", "selector".
- `npm test`, `tsc`, and clippy pass; screenshots refreshed in `apps/open-stream-hub/qa/`.

## What was done
- **Placeholders removed.** Deleted the disabled "Flows · Planned" nav item (`App.tsx`) and the inspector's "Run flow / Run script — Planned" block (`Inspector.tsx`), with unused icon imports and `.planned-actions` CSS cleaned out. The unreachable unsupported-module fallbacks were also reworded from "Planned" to "Not available yet".
- **Card sublabels neutral.** `ModuleCatalog` now subscribes to the device feed and shows Connected / Not connected per module; "Device test pending" / "Planned" are gone. Hardware-verification detail remains on the Compatibility page (its "Planned" capability labels stay there by design).
- **Jargon renamed.** Draft → "Unsaved changes" wording across headings, dialogs, shortcuts and the guide; "Replay a capture" → "Test with sample input" with a "Run test" button and module "samples"; result columns read "Would run" / "No action bound"; "Module control" → "Let OpenStreamHub drive the display" (setting title, notices, blockers, guide; option values App / Built-in animation unchanged); the serial "Device selector" now sits under an **Advanced** disclosure as "Limit to one device". Default-flow copy no longer mentions Rust, the service or selectors. Rust-side refusal strings and tests were left unchanged (backend, not UI).
- **Notes collapsed.** Timer, profile-switch and all OBS action notes in `ActionFields.tsx` are one-line hints with the details behind a "Learn more" disclosure; new `details.advanced` / `.learn-more` styles.
- **Tests and onboarding.** Updated assertions in 9 test files and added a catalog-label test (connected / not connected / no "Planned"). Onboarding try/profile/safety steps now match the renamed features.

## Verification
- `npx tsc --noEmit` clean; `npm test` 636/636 pass; `cargo clippy --workspace --all-targets` clean.
- Screenshots (browser preview at the native 1485×1087 size; "Desktop app required" banner expected outside the native app): `qa/copy-strip-devices-light.png`, `qa/copy-strip-devices-dark.png`, `qa/copy-strip-inspector-advanced-dark.png`, `qa/copy-strip-profiles-dark.png`, `qa/copy-strip-sample-input-dark.png`, `qa/copy-strip-display-setting-dark.png`. All six passed an independent visual review (no overlaps, clipping, theme or contrast problems; expected copy confirmed on screen).
- Dated section appended to `apps/open-stream-hub/design-qa.md` (file is gitignored, as is `qa/`).

## Review follow-ups (9 October 2026)
- Browser-preview banner now reads "Profile storage, validation and sample tests run in the native app." (`App.tsx`).
- README aligned with the renamed UI: "Settings > Let OpenStreamHub drive the display", "module samples" / "sample tests" wording. Internal masterhub-* docs intentionally keep the "Module control" name — they document the `display-settings.json` / service terminology.
- `qa/copy-strip-devices-light.png` and `qa/copy-strip-devices-dark.png` re-captured with the updated banner; tsc clean, 636/636 tests pass.

