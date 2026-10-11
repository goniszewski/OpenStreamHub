# Localisation

OpenStreamHub's text is moving from strings written in the components to message catalogues, one per language. English
is the source. Nothing is fetched at run time: the catalogues are part of the build, and switching the language redraws
the app at once with no restart.

## How it works

- `apps/open-stream-hub/src/locales/en.ts` holds every message by key. A message is text with `{name}` places for values,
  or, for words that change with a count, one text per plural category (`one`, `few`, `many`, `other`; `other` is
  required). Plural forms follow the language's own rules (`Intl.PluralRules`), so Polish has four.
- `locales/pl.ts` is typed to have exactly English's keys, so a missing or extra translation is a compile error.
- `i18n.ts` has the pure parts (choosing the locale from the system's languages, filling places, picking plural forms,
  falling back from the language to English to the key) and the store that holds the chosen language. `i18n-react.ts` has
  the app's store and the `useT()` hook components call: `const t = useT(); t('nav.workspace')`, or with values:
  `t('drawer.connected', { count })`.
- Settings has a **Language** picker: System default (the first language of the Mac that the app has, else English) or a
  language named in itself. The choice is remembered locally, and `<html lang>` follows it.
- Tests check that every catalogue has the same keys, that places match between languages, that every plural has the forms
  its language needs, and that the components already converted contain no hard-coded visible text.

## What is converted

The left navigation, the modules drawer, the change-history list and its button, the header's tooltips and device status,
the appearance names and the Settings page's title, appearance and language sections. Everything else is still written
in the components in English, so a Polish run shows those parts in English until they move over. Converted components are
listed in `i18n-components.test.tsx`; add a file there when it is converted, and the test then stops new hard-coded text
creeping back in.

## Adding a language

1. Copy `locales/pl.ts` to `locales/<code>.ts`, type it as `Record<MessageKey, Message>`, and translate every message.
2. Add the code to `Locale` and an entry to `languages` in `i18n.ts`, and to `catalogues` in `i18n-react.ts`.
3. Run `npm test`: the catalogue tests tell you what is missing or mismatched.

Text that comes from the user (profile names, key labels, anything typed) is never translated.
