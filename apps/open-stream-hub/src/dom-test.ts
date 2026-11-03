// A small DOM harness for component tests that press, type and click: renders into jsdom and wraps every interaction in
// act() so effects and state settle before the next assertion. Test files using it start with `// @vitest-environment jsdom`.
import { act } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom lays nothing out, so scrolling is a no-op there.
if (typeof Element !== 'undefined') Element.prototype.scrollIntoView = () => {};

export interface Rendered {
  container: HTMLElement;
  $: <T extends Element = HTMLElement>(selector: string) => T;
  $$: <T extends Element = HTMLElement>(selector: string) => T[];
  rerender(node: ReactNode): void;
  click(target: Element): void;
  key(target: Element, key: string): void;
  pointerDown(target: Element | Document): void;
  unmount(): void;
}

export function render(node: ReactNode): Rendered {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root: Root = createRoot(container);
  act(() => root.render(node));
  const query = <T extends Element>(selector: string): T => {
    const found = container.querySelector<T>(selector);
    if (!found) throw new Error(`nothing matches ${selector}`);
    return found;
  };
  return {
    container,
    $: query,
    $$: <T extends Element>(selector: string) => [...container.querySelectorAll<T>(selector)],
    rerender: next => act(() => root.render(next)),
    click: target => act(() => { (target as HTMLElement).click(); }),
    key: (target, key) => act(() => { target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })); }),
    pointerDown: target => act(() => { target.dispatchEvent(new Event('pointerdown', { bubbles: true })); }),
    unmount: () => { act(() => root.unmount()); container.remove(); },
  };
}
