// Reads the markup of SelectField in static-markup tests: the trigger button and the (closed, hidden) listbox of choices.
const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

export interface RenderedChoice { value: string; label: string; selected: boolean; disabled: boolean }

// The choices of the select with this trigger id, in order.
export function choicesOf(html: string, id: string): RenderedChoice[] {
  const list = new RegExp(`<ul id="${escape(id)}-list"[^>]*>(.*?)</ul>`, 's').exec(html)?.[1] ?? '';
  return [...list.matchAll(/<li [^>]*data-value="([^"]*)" aria-selected="(true|false)"([^>]*)><span class="select-option-label">(.*?)<\/span>/gs)]
    .map(([, value, selected, rest, label]) => ({ value, label, selected: selected === 'true', disabled: rest.includes('aria-disabled="true"') }));
}

// What the closed select shows, or null when no such select is rendered.
export function shownBy(html: string, id: string): string | null {
  return new RegExp(`<button type="button" id="${escape(id)}" class="select-trigger"[^>]*><span class="select-value">(.*?)</span></button>`, 's').exec(html)?.[1] ?? null;
}

// The attributes of the trigger button of the select with this id (empty when none is rendered).
export function triggerOf(html: string, id: string): string {
  return new RegExp(`<button type="button" id="${escape(id)}" class="select-trigger"([^>]*)>`).exec(html)?.[1] ?? '';
}

// True when some select offers this value (with this label unless null, and in that selected state when given).
export function hasOption(html: string, value: string, label: string | null, selected?: boolean): boolean {
  const pattern = new RegExp(`<li [^>]*data-value="${escape(value)}" aria-selected="(true|false)"[^>]*><span class="select-option-label">${label === null ? '' : `${escape(label)}</span>`}`, 'g');
  return [...html.matchAll(pattern)].some(match => selected === undefined || (match[1] === 'true') === selected);
}
