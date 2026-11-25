import { useCallback, useEffect, useId, useRef, useState } from 'react';
import type { FocusEvent, PointerEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { ControlTip } from './control-tooltip-model';

export const tooltipDelayMs = 350;
const halfWidth = 140;
const gap = 8;
const estimatedHeight = 140;

interface Anchor { control: number; centre: number; top: number; bottom: number }

// Where the tooltip sits: centred under the control, flipped above it near the bottom edge, kept inside the window sideways.
// It is positioned in window coordinates and drawn in a portal, so a turned or clipped stage cannot rotate or cut it.
export function tooltipPlacement(anchor: Pick<Anchor, 'centre' | 'top' | 'bottom'>, viewport: { width: number; height: number }) {
  const left = Math.min(Math.max(anchor.centre, halfWidth + gap), Math.max(viewport.width - halfWidth - gap, halfWidth + gap));
  const above = anchor.bottom + gap + estimatedHeight > viewport.height && anchor.top - gap - estimatedHeight > 0;
  return above ? { left, bottom: viewport.height - anchor.top + gap } : { left, top: anchor.bottom + gap };
}

// Hover/focus tooltips for the controls of one stage. `bind(control)` gives the props a control element spreads;
// `node` is the tooltip to render once. Hidden by leaving, blurring, pressing the control (a fader drag) or Escape.
export function useControlTooltip(resolve: (control: number) => ControlTip) {
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const base = useId();
  const hide = useCallback(() => { clearTimeout(timer.current); setAnchor(null); }, []);
  const show = useCallback((control: number, element: Element) => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      const rect = element.getBoundingClientRect();
      setAnchor({ control, centre: rect.left + rect.width / 2, top: rect.top, bottom: rect.bottom });
    }, tooltipDelayMs);
  }, []);
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!anchor) return;
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') hide(); };
    document.addEventListener('keydown', onKey);
    window.addEventListener('scroll', hide, true);
    return () => { document.removeEventListener('keydown', onKey); window.removeEventListener('scroll', hide, true); };
  }, [anchor, hide]);

  const idFor = (control: number) => `${base}-tip-${control}`;
  const bind = (control: number) => ({
    'aria-describedby': anchor?.control === control ? idFor(control) : undefined,
    onPointerEnter: (event: PointerEvent<HTMLElement>) => { if (event.pointerType !== 'touch') show(control, event.currentTarget); },
    onPointerLeave: hide,
    onPointerDown: hide,
    onFocus: (event: FocusEvent<HTMLElement>) => { if (event.currentTarget.matches(':focus-visible')) show(control, event.currentTarget); },
    onBlur: hide,
  });
  const node: ReactNode = anchor && typeof document !== 'undefined' ? createPortal(<ControlTipCard id={idFor(anchor.control)} tip={resolve(anchor.control)} anchor={anchor} />, document.body) : null;
  return { bind, node };
}

function ControlTipCard({ id, tip, anchor }: { id: string; tip: ControlTip; anchor: Anchor }) {
  const place = tooltipPlacement(anchor, { width: window.innerWidth, height: window.innerHeight });
  return <div id={id} role="tooltip" className="control-tip" style={place}>
    <strong className="control-tip-title">{tip.title}</strong>
    {tip.rows.map((row, index) => <div key={index} className="control-tip-row">
      <span className="control-tip-name">{row.gesture ? <span className="control-tip-gesture">{row.gesture}</span> : null}{row.name}</span>
      <span className="control-tip-description">{row.description}</span>
    </div>)}
  </div>;
}
