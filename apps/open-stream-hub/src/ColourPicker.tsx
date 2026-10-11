import { useEffect, useRef, useState } from 'react';
import type { PointerEvent, ReactNode } from 'react';
import { normaliseHexColour } from './lighting-model';
import { colourPresets, hexToHsv, hsvToHex } from './colour-math';

// An inline picker with exact values: a saturation/brightness square, a hue slider, a hex field and
// preset swatches. The system picker's swatches are not pure channels (its red is #ff2600).
export function ColourPicker({ id, value, disabled, onChange, children }: { id: string; value: string; disabled?: boolean; onChange(hex: string): void; children?: ReactNode }) {
  // The hue is kept apart from the hex, which loses it for greys and black.
  const [hsv, setHsv] = useState(() => hexToHsv(value));
  const [text, setText] = useState(value);
  const square = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (hsvToHex(hsv) !== value) setHsv(previous => { const next = hexToHsv(value); return next.s === 0 || next.v === 0 ? { ...next, h: previous.h } : next; });
    setText(value);
    // Only an outside change of the value re-seeds the picker.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  function commit(next: { h: number; s: number; v: number }) {
    setHsv(next);
    const hex = hsvToHex(next);
    setText(hex);
    if (hex !== value) onChange(hex);
  }
  function drag(event: PointerEvent<HTMLDivElement>) {
    const box = square.current?.getBoundingClientRect();
    if (disabled || !box || box.width === 0) return;
    const clamp = (n: number) => Math.min(1, Math.max(0, n));
    commit({ h: hsv.h, s: clamp((event.clientX - box.left) / box.width), v: 1 - clamp((event.clientY - box.top) / box.height) });
  }
  function typed(next: string) {
    setText(next);
    const hex = normaliseHexColour(next);
    if (hex) { setHsv(previous => { const parsed = hexToHsv(hex); return parsed.s === 0 || parsed.v === 0 ? { ...parsed, h: previous.h } : parsed; }); if (hex !== value) onChange(hex); }
  }
  return <div className="colour-picker">
    <div ref={square} className="colour-square" role="presentation" style={{ background: `linear-gradient(to top,#000,transparent),linear-gradient(to right,#fff,hsl(${hsv.h} 100% 50%))` }}
      onPointerDown={event => { event.currentTarget.setPointerCapture?.(event.pointerId); drag(event); }}
      onPointerMove={event => { if (event.buttons === 1) drag(event); }}>
      <span className="colour-handle" style={{ left: `${hsv.s * 100}%`, top: `${(1 - hsv.v) * 100}%`, background: value }} />
    </div>
    <input className="colour-hue" type="range" min={0} max={360} step={1} aria-label="Hue" value={Math.round(hsv.h)} disabled={disabled}
      onChange={event => commit({ ...hsv, h: Number(event.target.value) })} />
    <div className="colour-presets" role="group" aria-label="Preset colours">
      {colourPresets.map(preset => <button key={preset.hex} type="button" className="swatch" style={{ background: preset.hex }} title={`${preset.name} ${preset.hex}`}
        aria-label={`Use ${preset.name.toLowerCase()} ${preset.hex}`} disabled={disabled} onClick={() => { setHsv(hexToHsv(preset.hex)); setText(preset.hex); if (preset.hex !== value) onChange(preset.hex); }} />)}
    </div>
    <div className="colour-footer">
      <input id={id} className="colour-hex" type="text" inputMode="text" spellCheck={false} maxLength={7} aria-label="Colour as hex" value={text} disabled={disabled}
      onChange={event => typed(event.target.value)} onBlur={() => setText(value)} />
      {children}
    </div>
  </div>;
}
