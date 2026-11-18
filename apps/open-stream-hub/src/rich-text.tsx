import { Fragment } from 'react';
import type { ReactNode } from 'react';

// A message with its {name} places filled by nodes rather than text, and its `backticked` parts shown as inline code.
// Only the message's own text is read for backticks, never a value, so a path or command holding one stays as it is.
// `t(key)` without values returns the message with its places intact, which is what this takes.
export function richText(message: string, values: Record<string, ReactNode> = {}): ReactNode {
  return message.split(/(\{\w+\}|`[^`]+`)/).map((part, index) => {
    const place = /^\{(\w+)\}$/.exec(part);
    if (place) return <Fragment key={index}>{place[1] in values ? values[place[1]] : part}</Fragment>;
    if (part.length > 2 && part.startsWith('`') && part.endsWith('`')) return <code key={index}>{part.slice(1, -1)}</code>;
    return part;
  });
}

// Items as inline code, separated by commas.
export function codeList(items: readonly string[]): ReactNode {
  return items.map((item, index) => <Fragment key={index}>{index > 0 && ', '}<code>{item}</code></Fragment>);
}
