# 09 · Lighting panel in the app

**Priority:** P1 · **Effort:** M (dial) / L (all modules) · **Hardware needed:** yes for non-dial modules

**Done · 10 October 2026 · commits 8dd39bc and 753698a on master (pushed) · in-app hardware check on the M131E still pending, recorded in `docs/masterhub-hardware-results.md`**

## Why
Official has a Lighting tab: Single module / Whole kit, effects (Off, Static, Breathing, Strobe, Cycling, Random, Wave, Water, Rainbow), colour, brightness, speed, favourites. We have dial backlight + solid colour in the CLI only, verified on a real M131E.

## Scope
- Phase 1: dial backlight (0–100) and solid colour in the app, with live apply and a "Reset" button.
- Phase 2 (hardware-blocked): knob rings, base edge, faders; effects/speed once protocol is known.
- Colour picker: native input + favourites; per-module vs whole-kit toggle.
- Optional lighting action (next/previous effect, brightness up/down) once phase 2 lands.

## Acceptance
- Phase 1: round-trips through a fake transport in tests; hardware check recorded in `masterhub-hardware-results.md`.
- Unsupported modules show disabled controls with a reason, not hidden.

## What was done

Phase 1 only (the dial). A new **Lighting** page (sidebar, ⌘3; Compatibility moved to ⌘4, Settings to ⌘5) applies the encoder dial's backlight and one solid colour to the hardware as the controls move.

- **Rust** (`src-tauri/src/lighting.rs`): `write_dial_lighting` sends the three feature reports in the order verified on hardware — software mode (`03 49 01`, so the writes take effect; the built-in animation stops and the screen stays dark until a run draws), then backlight (`03 44`, 0–100, validated before any write), then colour (`03 46`, R, G, B). `restore_dial_mode` (`03 49 00`) is the Reset: the dial's own animation returns. Targets resolve through the shared `select_device`; each must name exactly one inventory entry and an M131E, and every target is opened before the first write so a wrong target never half-applies. Commands `set_dial_lighting`/`reset_dial_lighting` (registered in the generate_handler!/build.rs/capabilities triple) refuse targets a running profile owns (`service::check_lighting_allowed`, the feed rule) and return the number of dials written.
- **Frontend** (`LightingPage.tsx` + `lighting-model.ts`): scope toggle This module / Whole kit (every connected dial); a module picker listing all connected modules; backlight slider 0–100; native `<input type="color">` plus favourite swatches (localStorage `open-stream-hub.lighting-favourites`, newest first, 12 max, remove buttons); live apply debounced 250 ms through `applyLighting`; Reset button. Everything decidable — targets, gating, favourites, request shaping — is pure in `lighting-model.ts`; the page holds controls only and never applies on mount (opening the page leaves the dial alone).
- **Disabled with a reason, not hidden:** the controls always render. The reasons, in gate order: browser preview, a run driving a target dial (the frontend checks `isActivePath`; the backend refuses again), no modules connected, Whole kit with no dial, and a picked module that cannot be lit — that one names the module ("3-knob module lighting is not available yet. The encoder dial is the only module OpenStreamHub lights right now."). A one-line hint plus a Learn more disclosure explains the software-mode side effect, Reset, and what phase 2 does not cover yet.
- **QA seeding:** dev-only `window.__oshLightingSeed(devices)` (App.tsx → `DeviceFeed.seedPreview`) so browser-preview captures can exercise the page; auto-feeds on seeded paths no-op in the preview.

## Verification

- `npx tsc --noEmit` clean; vitest suites all green. New tests: `lighting-model.test.ts` (targets per scope, every gate reason, picker labels, hex normalisation, R/G/B channels, backlight clamp, apply/reset invoke shapes with a fake invoke, favourite round-trip/dedupe/cap, status wording) and `lighting-page.test.tsx` (usable controls for a dial; preview reason with locked fieldset; empty-kit, unsupported-module and run-owned reasons rendered; favourites from storage). `shortcuts.test.ts` updated for ⌘3–5.
- `cargo test` in the app workspace all green. New tests: `lighting::tests` (fake transport: exact 32-byte mode/backlight/colour reports in the verified order and zero padding; out-of-range backlight writes nothing; a transport failure stops at the failing write; Reset sends only `03 49 00`; dial selection accepts one M131E path and refuses other modules, missing and ambiguous paths) and `service::tests::lighting_writes_are_refused_while_a_service_owns_the_interface`. The generate_handler!/build.rs/capabilities triple is enforced by the existing lib.rs test. `cargo fmt --check` clean in both workspaces; `cargo clippy --workspace --all-targets` clean.
- Captures in `qa/` (gitignored, entries in `qa/design-qa.md`): `lighting-page-dark.png` (enabled panel with favourites; devtools-lifted controls noted there) and `lighting-unsupported-dark.png` (module named, controls disabled). The first capture pass found App was not passing `storage` to the page, which would have made favourites never load — fixed before capture.
- Docs synced: `docs/masterhub-implementation-status.md` (Lighting page bullet; the stale "color channel order remains unverified" clause corrected to the hardware-verified R, G, B), `docs/masterhub-hardware-results.md` (pending in-app lighting check recorded, next to the CLI-verified facts), app `README.md` (Lighting feature bullet, ⌘1–5), Compatibility page (Dial lighting entry under Software and modes).

## Close condition

The acceptance's fake-transport round-trip is done; the hardware check is recorded as **pending** in `docs/masterhub-hardware-results.md` ("Still pending: the app's Lighting page"): someone with the M131E connected needs to see a live apply change the dial's lighting, Whole kit reach every dial, a run hold the write off, and Reset return the built-in animation. The CLI-level facts the page builds on (software mode, backlight read-back, R, G, B order, animation returning on `03 49 00`) were verified on 8 October. Per the task 04 convention this file stays in review until that check is done or the task is knowingly closed with the follow-up.

## Review round · 10 October 2026

The reviewer's two findings, fixed in commit 753698a:

1. **The last slider value could be silently dropped.** `applyNow` returned early while a write was still in flight and nothing re-scheduled it, so a change landing during a slow write (likely with Whole kit on several dials) never shipped and the dial kept a value the controls no longer showed. Applies now run through `createApplyQueue` (`lighting-model.ts`): one write in flight, and a mid-flight change marks exactly one trailing apply that re-reads the latest values and gate when the write finishes. The page creates the queue per mount cycle (StrictMode's effect/cleanup/effect always leaves a live queue) and disposes it on unmount, so no hardware write happens after the page closes. Tests: a slow fake invoke with a mid-flight change asserts the last value is sent; several mid-flight changes coalesce into one trailing apply; a burst debounces to one; dispose stops scheduling and firing (`lighting-model.test.ts`).
2. **The Learn more text contradicted the run lock.** It said a run-driven dial was "left out" of a Whole kit apply, but the gate disables the whole page and the backend refuses the write. The copy now reads "While a run is driving any dial, Whole kit is unavailable. Stop the run to change lighting.", and `lighting-page.test.tsx` pins the corrected sentence and rejects the old one.

Re-verified after the fixes: `npx tsc --noEmit` clean and all vitest suites green (the new queue tests included). No Rust code changed, so the cargo suites, fmt and clippy results from 8dd39bc stand.

Open items unchanged: the in-app hardware check on the M131E stays pending (`docs/masterhub-hardware-results.md`) — it needs eyes on the physical dial, so it remains for the next hardware session; the commits are unpushed (pushed only on request, per convention). Task 04's real-OBS 28+ check is likewise still waiting and untouched by this round.
