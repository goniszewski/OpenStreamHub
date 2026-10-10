# 10 · Dial functions and display modes in the dial inspector

**In review · 10 October 2026 · commits 6589a96 and 4e7d39d on master (local, not pushed)**

## Why
Official dial panel has Edit / Sort / LCD Display tabs: ordered function list (press cycles), drag-sort, display mode (Default, CPU/GPU, Clock Date/Analog, Image). Ours is `DialFunctionsPanel` buried on the Profiles page, and the dial face is behind an experimental Settings toggle.

## Scope
- Move dial functions into the dial inspector with tabs: Functions (add, remove, drag-sort) · Display (mode picker, clock style, image upload).
- Show the generated dial face as the preview inside the module image (extends `EncoderDisplayContent`).
- Promote dial display from experimental toggle to a per-profile setting once hardware acceptance in `masterhub-dial-functions.md` is done.
- Show only CPU usage (GPU/temperature are out of scope per parity tracker).

## What was done

**Profile schema (service crate).**
- `DialDisplay { content, image? }` in `crates/cooltek-sdk-service/src/dial.rs`, with `DialDisplayContent` = `off | function | clock | analog_clock | cpu | image` — the same values as `masterhub-service --dial-content`, enforced by a parity test in the app. `Profile` gains the optional additive field `dial_display` (skipped when absent, like `dial_functions`; no schema 3, old readers reject instead of silently ignoring). Validation (`Profile::validate` → `dial::validate_display`): the picture is required exactly for image content, capped at 64 KiB decoded (text pre-check + decoded check) and must be a 240×240 baseline JFIF JPEG (`dial::validate_jpeg`); anything else is a load/save error, never a silently wrong face.
- Absent means today's default: the active dial function's name — or the analog clock when the profile has nothing on the dial (no functions, no M131E binding), because there would be no name to show. The fallback logic moved from the settings into the per-profile path (`display::dial_content` in src-tauri).

**Dial artwork (CLI crate).**
- `DialContent::Image` (`--dial-content image`): `DialArtwork::prepare` decodes the profile's picture once and re-validates it before a run ("fails before a run", like every generated face); `dial_image` serves the exact stored bytes under the static `image` request key. `base64` moved to a real dependency of the CLI crate (it decoded profile artwork only in tests before).

**App backend (src-tauri).**
- `display.rs`: the global `dial_content` setting is gone from `DisplaySettings` (now `{module_control, auto_execute}`). `display-settings.json` still reads the two earlier shapes — the experimental toggle's `{"enabled", "dial_content"}` and the three-field shape — with the dial choice ignored (`IgnoredAny`); the next save writes the current shape. `display_args` derives `--display-sync --dial-content <value>` from the running profile's `dial_display` (off → no flags), so a profile switch between faces follows each spawn, exactly like the keypad's flat/page switch. Import of official `.mhz` files leaves `dial_display` empty (their mode is not mapped).
- Shared IPC fixture gains the serialized `dial_display` shapes and the six content values (`display-settings.json`, regenerated and pinned by both sides' tests).

**App frontend.**
- The encoder inspector's dial editor is a two-tab card (`DialEditorTabs` in `Inspector.tsx`): **Functions** (the unchanged `DialFunctionsPanel`, including its no-functions offer below the flat editor) and **Display** (new `DialDisplayPanel`). Tab state is view-only.
- `DialDisplayPanel`: mode picker with the six faces (clock style is choosing between the digital and analog entries, matching the existing wording), the CPU honesty note (measured on this Mac, illustrative preview value), and for image mode the stored-picture thumbnail with Remove image, a file picker, and the note "Preparing the dial picture needs the desktop app." Uploads run the existing `prepare_artwork` command with the dial target (240×240), lowering JPEG quality (85→25) until the picture fits the 64 KiB profile budget; results never apply across a draft change (the task-16 `draftGeneration` guard).
- `dial-face.ts`: a pixel-faithful port of the `dial_artwork.rs` rasteriser (same palette, 5×7 glyphs, seven-segment digits, ring/spoke/disc fills and name wrapping) painting 240×240 faces on a canvas; `DialFace` (replacing `EncoderDisplayContent`) shows the profile's effective face in the module hero image, the rig tile and the Display tab — generated faces on canvas, the uploaded picture as the stored JPEG itself, nothing when off, clock faces following the minute. Faces are clipped to the round screen. The old action-label display on the dial screen is gone (the face is what a run draws, not the selected binding).
- Settings keeps only Module control and Run actions automatically; its note now says the dial screen shows what the running profile picks in its dial editor. A dev-only `window.__oshDialDisplaySeed` preview hook joins the other QA seeds.

## Verification
- `cargo test --workspace` green (service 220, CLI 187 + integration, app 380), clippy clean, `cargo fmt` applied. New tests: profile `dial_display` parse/round-trip/refusals (strict JSON, image required only for image, 64 KiB cap by text and by decoded size, real 240×240 JPEG accepted), `DialContent::Image` parse and serve (exact stored bytes, refuses a profile without a picture), settings-file migration (both earlier shapes, strict current shape, `UNREADABLE` unchanged), `display_args` per profile (idle analog-clock fallback, function name with a binding or functions, explicit choices kept, off → no flags, CLI value parity).
- Frontend: `npm run check` clean and vitest 836 green. New `dial-display-model` and `DialDisplayPanel` suites (picker, upload gating, honest CPU note, tab wiring as a real tablist), `DisplayContent` dial-face pixel tests (device-palette colours and name wrapping), rewritten `display-settings` (2-field contract, profile-driven `displayFlags`, fixture pin of the serialized `dial_display`), and updated `RigPreview` (the dial screen shows the face, not the binding label).
- QA captures (browser preview, dark, 1485×1087; canvas `getImageData` pixel probes): `qa/dial-display-dark.png` (Display tab, default profile face in hero/rig/preview), `qa/dial-display-clock-dark.png` (Analog clock through the real picker; centre pixel `[230,120,40]` = the device's hand orange), `qa/dial-display-image-dark.png` (Custom image seeded via the new preview hook, thumbnail/Remove/file picker and the desktop-app note), `qa/display-settings-dark.png` (Settings without the dial select). Dated entry in `design-qa.md`.
- Docs: `masterhub-display-sync.md` gains "The profile's dial display setting" and the `image` face/requests; `masterhub-service.md` flag table and records updated; `masterhub-dial-functions.md` checklist items 8 and 10 and the live-limitation note updated; the pending on-hardware check recorded in `masterhub-hardware-results.md`.

## Acceptance
- Existing `dial_functions` profiles load unchanged (round-trip tests; the field is additive and skipped when absent) — done.
- Preview face matches `dial_artwork.rs` output (the preview is the same rasteriser ported, with pixel tests and hardware-palette probes) — done.
- The dial faces already ran on a real M131E (8 October); the `image` face and the app's edit-then-run flow of the new setting have not. Recorded as pending in `masterhub-hardware-results.md`; the task leaves in-review once that check has run there and its result is logged.
