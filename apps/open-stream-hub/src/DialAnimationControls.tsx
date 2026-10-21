import { useEffect, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import type { DialAnimation } from './contracts';
import { animationRateLabel, averageFrameLabel, dialPresets, sameAnimation, stepIndex } from './dial-animation-model';
import type { DialPreset } from './dial-animation-model';
import { keyArtworkFrame } from './key-artwork';

// A prepared animation playing with its own frame times, so each variant can be judged by eye.
export function AnimatedPreview({ animation, size }: { animation: DialAnimation; size: number }) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(0);
    const start = performance.now();
    const timer = window.setInterval(() => setElapsed(performance.now() - start), 50);
    return () => window.clearInterval(timer);
  }, [animation]);
  const frame = animation.frames[keyArtworkFrame(animation, elapsed)] ?? animation.frames[0];
  return <img src={`data:image/jpeg;base64,${frame.jpeg_base64}`} alt="" width={size} height={size} draggable={false} />;
}

export type DialVariant = { animation: DialAnimation } | { error: string };
export type DialVariants = Record<DialPreset, DialVariant>;

// The three versions of an imported GIF, one at a time and playing, with arrows (or the left and right keys) to look at
// the next. Looking changes nothing: Use puts the shown version in the profile. A version the GIF cannot reach (too
// detailed for its frame budget) can be looked at but not used.
export function DialVariantPicker({ variants, current, onChoose, disabled }: { variants: DialVariants; current: DialAnimation | undefined; onChoose(preset: DialPreset): void; disabled: boolean }) {
  const inUse = Math.max(0, dialPresets.findIndex(preset => { const variant = variants[preset.id]; return 'animation' in variant && sameAnimation(variant.animation, current); }));
  const [shown, setShown] = useState(inUse);
  // Following the profile: a version chosen elsewhere (an undo, a new GIF) is the one to look at.
  useEffect(() => { setShown(inUse); }, [inUse, current]);
  const step = (delta: number) => setShown(index => stepIndex(index, delta, dialPresets.length));
  const preset = dialPresets[shown];
  const variant = variants[preset.id];
  const ok = 'animation' in variant;
  const used = ok && sameAnimation(variant.animation, current);
  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'ArrowLeft') { event.preventDefault(); step(-1); }
    else if (event.key === 'ArrowRight') { event.preventDefault(); step(1); }
  };
  return <section className="dial-variants" aria-roledescription="carousel" aria-label="Quality and speed of the GIF" onKeyDown={onKey}>
    <div className="dial-variants-head">
      <h4>Quality and speed</h4>
      <div className="dial-variants-nav">
        <button type="button" className="button compact" aria-label="Previous version" disabled={disabled} onClick={() => step(-1)}><ChevronLeft size={16} /></button>
        <span className="small-label" aria-hidden="true">{shown + 1} of {dialPresets.length}</span>
        <button type="button" className="button compact" aria-label="Next version" disabled={disabled} onClick={() => step(1)}><ChevronRight size={16} /></button>
      </div>
    </div>
    <div className="dial-variant" role="group" aria-roledescription="slide" aria-label={`${shown + 1} of ${dialPresets.length}: ${preset.label}`} aria-live="polite">
      {ok ? <AnimatedPreview animation={variant.animation} size={128} /> : <span className="dial-variant-missing" aria-hidden="true" />}
      <div className="dial-variant-text">
        <span className="dial-variant-name">{preset.label}{used && <span className="dial-variant-badge">In use</span>}</span>
        <span className="fine-print">{ok ? `${animationRateLabel(variant.animation)} · ${averageFrameLabel(variant.animation)}` : 'Too detailed for this setting.'}</span>
        <span className="fine-print">{preset.help}</span>
        {!used && <button type="button" className="button compact" disabled={disabled || !ok} onClick={() => onChoose(preset.id)}>Use {preset.label}</button>}
      </div>
    </div>
  </section>;
}
