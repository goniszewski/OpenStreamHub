## Summary
- The If / Set variable editors no longer show a made-up "Is equal to: Off" when no variable exists, and link to the Variables section.
- All selects use a styled listbox dropdown instead of the unstyleable native popup.
- Each module shows "x of y assigned" with a popup listing every hardware trigger and its action.

## Changes
- Value fields stay hidden until a variable is chosen; "Add one under Variables" opens and focuses the Variables section.
- `SelectField` is now a select-only combobox (keyboard, type-ahead, ARIA) with the same `<option>` children API; all selects and tests migrated.
- `moduleAssignments` counts controls in use, folders included and the reserved Back key excluded on folder pages.
- `ModuleAssignmentsChip` popup closes on outside click, Escape, and module or keypad page change.

## Test plan
- [x] `tsc` and the full vitest suite pass (1354 tests)
- [ ] Open a dropdown near the bottom of the inspector and check it is not clipped
- [ ] Click and keyboard use of the dropdown and the assignments popup in the built app
- [ ] Add an If with no variables: no value field, link focuses Add variable
