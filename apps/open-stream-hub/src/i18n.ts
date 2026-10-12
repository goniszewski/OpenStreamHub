// The app's message layer: the English catalogue, and `translate`, which picks a message and fills in its values. The
// app is English only. Messages live under keys (not in the components) so wording is kept in one place and the
// components that use them are checked for stray hard-coded text. Nothing is fetched at run time.

// A message is text with {name} places for values, or one text per plural category (`other` is required).
export type PluralMessage = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };
export type Message = string | PluralMessage;

// Fills {name} places. A place with no value stays as written, so a missing value shows up in testing rather than
// vanishing; a value that is a number is shown as given (no digit grouping), which suits counts.
export function fill(text: string, values: Record<string, string | number> = {}): string {
  return text.replace(/\{(\w+)\}/g, (place, name: string) => name in values ? String(values[name]) : place);
}

// The text of a message for these values: a plural message picks its form by `count`.
export function format(message: Message, values?: Record<string, string | number>): string {
  if (typeof message === 'string') return fill(message, values);
  const count = Number(values?.count);
  const category = Number.isFinite(count) ? new Intl.PluralRules('en').select(count) : 'other';
  return fill(message[category] ?? message.other, values);
}

// Looks a key up in the catalogue, falling back to the key itself.
export function translate(catalogue: Record<string, Message>, key: string, values?: Record<string, string | number>): string {
  const message = catalogue[key];
  return message === undefined ? key : format(message, values);
}
