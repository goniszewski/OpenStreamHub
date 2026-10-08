# Nested keypad folders

Design recorded on 5 October 2026 for OpenStreamHub's device-free editor and simulation. Physical screen updates remain a separate hardware milestone.

## Behavior

A profile's root keypad page has 15 configurable keys. A key can contain an action or open a folder. Each folder displays 14 content keys and a generated Back key in the top-left position. Folders can contain other folders, up to eight levels deep.

The Back key shows a large left arrow and a small `L1`, `L2`, … `L8` badge for the **current** depth. The root is depth zero and has no generated Back key. Back moves up one level; returning to the root restores its ordinary first key. The desktop breadcrumb shows the full path and permits navigation to an ancestor.

Folder navigation changes keypad content within the active profile. Other modules retain their existing bindings. Switching or reloading a profile starts at its root. Folder names and contents are saved with the profile; the temporary navigation path is not persisted.

Creation, renaming, action editing, and clearing operate on the current page. Clearing a populated folder requires an in-app confirmation and removes its descendants from the draft. Save is still explicit, and existing protections for unsaved drafts apply.

## Profile and runtime model

Schema 2 adds a keypad layout consisting of pages with stable IDs, names, and 15 explicit slots. The root ID is `root`. Each non-root page stores an empty slot zero; Back is generated at runtime and cannot be reassigned. Action tiles retain pressed/released bindings and serial-specific precedence. Folder tiles reference child page IDs.

Schema 1 remains readable. An explicit in-memory upgrade moves only its M4315 bindings into action tiles on the root page, preserving selectors, serials, and actions. Other module bindings retain their behavior. Opening an existing profile does not rewrite its file; saving uses the existing atomic storage path.

Validation rejects duplicate IDs, missing targets, cycles, multiple parents, unreachable pages, a folder targeting the root, editable Back slots, unsupported actions/gestures, and excessive size or depth. Limits are 64 pages, eight folder levels, the existing 256 total action bindings, and the existing 1 MiB JSON input limit.

Navigation belongs to the shared service runtime. Each press/release pair is associated with the page where it began. A folder or Back press can change the page, but its release cannot activate the tile that has appeared beneath the same physical key. Held action keys retain their original release action across navigation. Resetting a session clears held-key state and returns to the root.

The editor and simulator use the same validated profile format and runtime rules. Navigation previews may return a new path and planned action; they open no HID device and execute no host operation. Ordinary input replay should preserve unknown/malformed-report diagnostics.

## Windows reference and hardware boundary

The extracted official Windows software 1.0.0.113 supports one folder level. Its `src/store/module/masterHub.js` lines 36–40 exclude `new-folder` inside folders; `src/components/Widget/15keys.vue` lines 587–590 enforce the drop restriction, and `src/store/module/keypad.js` lines 2021–2023 reject equivalent paste operations. Folder slot zero is reserved, and `src/App.vue` lines 1222–1228 return it to the root. OpenStreamHub's multiple levels extend that behavior.

The inspected Windows paths change host-side mappings and redraw through ordinary per-key JPEG calls. They establish no separate firmware directory command. Existing [offline keypad image preparation and packet planning](masterhub-diagnostics.md#offline-keypad-image-preparation-and-plans) supply the image transport foundation. Physical key order, orientation, image acceptance, pacing, and responsive page transitions still require hardware testing. This milestone does not add physical display upload or host action execution to the desktop app.

## Acceptance checks

- Create a folder and a child folder, configure an action, save, reload, and recover the same hierarchy.
- Enter levels one and two; verify `L1`/`L2`, breadcrumbs, 14 content slots, and one-level Back navigation.
- Return to the root and recover its original first key and other module bindings.
- Verify navigation releases never dispatch a newly displayed child action, and held action releases retain their origin page.
- Reject malformed trees, reserved-slot edits, duplicate selectors, out-of-range controls, and excessive depth before saving or previewing.
- Preserve existing files during in-memory migration and failed validation.
- Inspect the native app in light and dark modes, including keyboard focus and unsaved-draft guards.

## Validation result

Device-free acceptance passed on Apple Silicon macOS on 5 October 2026. Automated checks cover malformed layouts, bounds, migration, navigation press/release pairing, held-origin releases, serial precedence and persistence. The rebuilt native app passed folder creation and naming through L3, per-page action editing, complete-click preview, explicit save/reload, breadcrumbs and Back, keyboard activation, light/dark inspection, subtree deletion and the unsaved guard, and root reset when switching profiles. Existing profile bytes and modification times were unchanged. On 8 October 2026 one real M4315 showed the first page of the example profile and redrew through folder navigation (two levels and back) with no write failures; see [display sync](masterhub-display-sync.md#hardware-results-m4315-8-october-2026). Key placement beyond keys 0 to 2 and the look of the redrawn pages remain unverified.
