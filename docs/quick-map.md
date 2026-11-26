# Quick map

The **Quick map** button in the app header opens a drawing of the open profile: a blueprint-style SVG (line art, numbered
callouts, a legend) meant purely as a reminder of what every key, knob, fader and dial does.

## What it shows

- Only the modules the profile has something bound on, in a fixed order: keypad, dial, knobs, faders, rollers. They flow
  left to right and wrap onto a second row. No hardware is needed; the drawing comes from the profile alone.
- The keypad is the **root page**: all 15 keys with their labels; empty keys are drawn dashed. A folder key shows a folder
  icon, and its legend line says it opens the folder (a sub-page). What is inside a folder is not drawn.
- Every bound control gets a numbered callout and a legend line, `Control → what it does`. Several gestures are joined
  (`Clockwise: Output volume +5 points; Press: Toggle output mute`); a key's press and a fader's position are left unnamed.
  The wording is `actionLabel` from `ui-model.ts`, the same text the bindings list uses. The dial lists each dial
  function's gestures (`Volume · Right: …`) and notes that a short press switches function.
- It draws the profile open in the editor. That follows the running profile (a focused-app rule or a profile key switches
  it) unless there are unsaved edits, in which case the map shows those edits.

## Saving and printing

On screen the drawing uses the app theme (light or dark). **Save as SVG**, **Copy SVG** and **Print** use a paper copy
(dark ink on white), so the file reads the same anywhere. Print opens the system print dialog for the drawing alone.
Saving uses a browser-style download from the app window; if the webview does not offer it, use **Copy SVG** and paste into
a file.

The dialog is a modal: Tab moves between the buttons and the scrollable sheet, Escape closes it. The SVG has a title and a
description that reads out the whole legend for screen readers.

## Where it lives

- `quick-map-model.ts`: the legend derivation (pure; tested in `quick-map-model.test.ts`).
- `QuickMap.tsx`: the drawing, the header button and the dialog (`QuickMap.test.tsx`).
- Messages: the `quickMap.*` keys in `locales/en.ts` (see [localisation](localisation.md)).
