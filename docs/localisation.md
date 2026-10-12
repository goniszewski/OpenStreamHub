# App messages

OpenStreamHub is English only; there is no language picker and no plan for other languages. Text that components show
is moving from strings written in the components to one message catalogue, so wording lives in one place.

## How it works

- `apps/open-stream-hub/src/locales/en.ts` holds every message by key. A message is text with `{name}` places for
  values, or, for words that change with a count, one text per plural category (`one`, `other`; `other` is required).
- `i18n.ts` has the pure parts: filling places, picking the plural form and falling back to the key. `i18n-react.ts`
  has the `useT()` hook components call: `const t = useT(); t('nav.workspace')`, or with values:
  `t('drawer.connected', { count })`. A message key that does not exist is a compile error.
- Tests check that no message is empty, that plurals have both forms, and that the components already converted
  contain no hard-coded visible text (`i18n-components.test.tsx`; add a file to its list when it is converted).

## What is converted

The left navigation, the modules drawer, the change-history list, the header's tooltips and device status, the run
controls, the profile manager parts, the starters, the inspector's test result and the demos. Everything else is still
written in the components.
