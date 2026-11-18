import type { CSSProperties, KeyboardEvent } from 'react';
import { ArrowDownToLine, ArrowUpFromLine, ChevronsDown, ChevronsUp, CircleDot, RotateCcw, RotateCw, SlidersVertical, Timer } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { Gesture } from './contracts';
import { gestureLabels } from './ui-model';
import { useT } from './i18n-react';

// One glyph per gesture. The encoder's Right/Left are its clockwise/counter-clockwise turns.
export const gestureIcons: Record<Gesture, LucideIcon> = {
  clockwise: RotateCw, counter_clockwise: RotateCcw, right: RotateCw, left: RotateCcw,
  pressed: ArrowDownToLine, released: ArrowUpFromLine, short_press: CircleDot, long_press: Timer,
  position_changed: SlidersVertical, up: ChevronsUp, down: ChevronsDown,
};

// The gesture picker: a radio group of icon tiles, one per gesture the control offers. A gesture
// with no action bound reads as unused (muted, no accent), so a glance shows what the control does.
// Arrow keys move the choice, as in any radio group; only the chosen tile is in the tab order.
export function GestureGrid({ id, label, gestures, value, bound, disabled, onChange }: {
  id: string; label: string; gestures: readonly Gesture[]; value: Gesture; bound(gesture: Gesture): boolean; disabled?: boolean; onChange(gesture: Gesture): void;
}) {
  const t = useT();
  const current = gestures.includes(value) ? value : gestures[0];
  const move = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const delta = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const next = gestures[(index + delta + gestures.length) % gestures.length];
    onChange(next);
    const group = event.currentTarget.parentElement;
    group?.querySelector<HTMLButtonElement>(`[data-gesture="${next}"]`)?.focus();
  };
  return <div id={id} role="radiogroup" aria-label={label} aria-disabled={disabled || undefined} className="gesture-grid"
    style={{ '--gesture-columns': gestures.length <= 3 ? Math.max(gestures.length, 1) : gestures.length === 4 ? 2 : 3 } as CSSProperties}>
    {gestures.map((gesture, index) => {
      const Icon = gestureIcons[gesture];
      const checked = gesture === current;
      const used = bound(gesture);
      return <button key={gesture} type="button" role="radio" data-gesture={gesture} aria-checked={checked} tabIndex={checked ? 0 : -1} disabled={disabled}
        className={`gesture-tile${checked ? ' checked' : ''}${used ? ' bound' : ' unbound'}`}
        onClick={() => onChange(gesture)} onKeyDown={event => move(event, index)}>
        <Icon size={18} aria-hidden="true" />
        <span className="gesture-name">{gestureLabels[gesture]}</span>
        <span className="sr-only">, </span><span className="gesture-status">{t(used ? 'inspector.gestureInUse' : 'inspector.gestureUnused')}</span>
      </button>;
    })}
  </div>;
}
