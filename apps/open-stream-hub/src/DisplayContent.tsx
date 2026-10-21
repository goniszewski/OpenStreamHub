import { useEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import { ArrowLeft, ArrowLeftRight, Camera, Clapperboard, Folder, Globe, Keyboard, ListOrdered, Monitor, Play, Timer, Type, Undo2, Volume2, VolumeX } from 'lucide-react';
import type { BindingAction, DialAnimation, DialDisplayContent, KeyArtwork, KeypadPage, KeypadSlot, Profile } from './contracts';
import { effectiveDialDisplay } from './dial-display-model';
import { dialFaceSide, glyph, nowDialTime, paintDialFace } from './dial-face';
import type { DialTime } from './dial-face';
import { keyArtworkFrame } from './key-artwork';
import { KeyArtworkPreview } from './KeyArtworkEditor';
import { actionLabel } from './ui-model';

export function ActionIcon({ action }: { action?: BindingAction }) {
  const Icon = action?.type === 'toggle_mute' ? VolumeX : action?.type === 'launch_app' ? Monitor :
    action?.type === 'volume' || action?.type === 'set_volume' || action?.type === 'volume_from_position' || action?.type === 'mic_input_volume_from_position' ? Volume2 :
    action?.type === 'open_url' ? Globe : action?.type === 'media' ? Play : action?.type === 'text' ? Type :
    action?.type === 'multi_action' ? ListOrdered : action?.type === 'profile' ? ArrowLeftRight : action?.type === 'timer' ? Timer : action?.type?.startsWith('obs_') ? Clapperboard : action?.type === 'system' ? (action.command === 'screenshot' ? Camera : Undo2) : Keyboard;
  return <Icon aria-hidden="true" />;
}

// The module draws a key label in its 5×7 bitmap font: large (two lines of up to 6 characters) when it fits that way,
// else small (three lines of up to 12), cut with a final dot. Mirrors `tile_artwork::lines_of`, so the preview wraps,
// sizes and shapes the text as the keys do.
function wrapLabel(label: string, width: number, lines: number): string[] | null {
  const wrapped: string[] = [];
  for (const word of label.split(/\s+/).filter(Boolean)) {
    if ([...word].length > width) return null;
    const last = wrapped.length - 1;
    if (last >= 0 && [...wrapped[last]].length + 1 + [...word].length <= width) wrapped[last] += ` ${word}`;
    else wrapped.push(word);
  }
  return wrapped.length <= lines ? wrapped : null;
}

export function keyLabelLines(label: string): { lines: string[]; scale: 1 | 2 } {
  const large = wrapLabel(label, 6, 2);
  if (large) return { lines: large, scale: 2 };
  const small = wrapLabel(label, 12, 3);
  if (small) return { lines: small, scale: 1 };
  const text = label.split(/\s+/).filter(Boolean).join(' ');
  const characters = [...text].slice(0, 36);
  const lines: string[] = [];
  for (let at = 0; at < characters.length; at += 12) lines.push(characters.slice(at, at + 12).join('').trim());
  if ([...text].length > 36) lines[lines.length - 1] = `${lines[lines.length - 1].slice(0, -1)}.`;
  return { lines, scale: 1 };
}

function KeyLabel({ text }: { text: string }) {
  const { lines, scale } = keyLabelLines(text);
  if (lines.length === 0) return null;
  const height = lines.length * 8 * scale - scale;
  const rects: ReactElement[] = [];
  lines.forEach((line, row) => {
    const characters = [...line];
    const left = (72 - (characters.length * 6 * scale - scale)) / 2;
    characters.forEach((character, index) => glyph(character).forEach((bits, y) => {
      for (let column = 0; column < 5; column += 1) {
        if (bits & (0b10000 >> column)) rects.push(<rect key={`${row}-${index}-${y}-${column}`} x={left + index * 6 * scale + column * scale} y={row * 8 * scale + y * scale} width={scale} height={scale} />);
      }
    }));
  });
  return <svg className="key-label" viewBox={`0 0 72 ${height}`} role="img" aria-label={text} fill="currentColor"><title>{text}</title>{rects}</svg>;
}

// The same content is used in the editor and the physical module illustration.
export function KeyDisplayContent({ slot, pages, depth, back, artwork }: { slot: KeypadSlot; pages: KeypadPage[]; depth: number; back: boolean; artwork?: KeyArtwork }) {
  if (back) return <><ArrowLeft aria-hidden="true" /><span className="display-depth">L{depth}</span><KeyLabel text="Back" /></>;
  if (artwork) return <KeyArtworkPreview artwork={artwork} />;
  if (!slot) return <span className="display-empty" aria-hidden="true">·</span>;
  if (slot.type === 'folder') return <><Folder aria-hidden="true" /><KeyLabel text={pages.find(page => page.id === slot.page_id)?.name ?? slot.page_id} /></>;
  return <><ActionIcon action={slot.bindings[0]?.action} /><KeyLabel text={slot.label} /></>;
}

// What the module illustration's dial screen shows: the face the running
// profile's dial display setting produces (the same painter the device gets,
// at module-image resolution), the uploaded picture itself, or nothing. The
// clock faces keep drawing as the minute turns; the CPU gauge shows one
// illustrative value, because a real reading only exists in a live run.
export function DialFace({ profile }: { profile: Profile }) {
  const content = effectiveDialDisplay(profile);
  if (content === 'off') return <span className="dial-face off" aria-hidden="true" />;
  if (content === 'image') {
    const animation = profile.dial_display?.animation;
    if (animation && animation.frames.length > 1) return <DialAnimatedFace animation={animation} />;
    const image = profile.dial_display?.image;
    // A validated profile always carries its picture; the blank frame only
    // shows while the editor holds an image mode without one yet.
    return image
      ? <img className="dial-face" src={`data:image/jpeg;base64,${image}`} alt="" draggable={false} />
      : <span className="dial-face off" aria-hidden="true" />;
  }
  return <DialGeneratedFace profile={profile} content={content} />;
}

// An imported GIF plays in the preview with the stored frame times, as the dial does.
function DialAnimatedFace({ animation }: { animation: DialAnimation }) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(0);
    const start = performance.now();
    const timer = window.setInterval(() => setElapsed(performance.now() - start), 50);
    return () => window.clearInterval(timer);
  }, [animation]);
  const frame = animation.frames[keyArtworkFrame(animation, elapsed)];
  return <img className="dial-face" src={`data:image/jpeg;base64,${frame.jpeg_base64}`} alt="" draggable={false} />;
}

// The illustration value shown on the CPU face; a live run draws the real
// reading instead.
const illustrativeCpuPercent = 37;

type GeneratedContent = Exclude<DialDisplayContent, 'off' | 'image'>;

function DialGeneratedFace({ profile, content }: { profile: Profile; content: GeneratedContent }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [time, setTime] = useState<DialTime>(() => nowDialTime());
  const clock = content === 'clock' || content === 'analog_clock';
  useEffect(() => {
    if (!clock) return;
    const timer = window.setInterval(() => {
      const next = nowDialTime();
      setTime(current => (current.hour === next.hour && current.minute === next.minute ? current : next));
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [clock]);
  useEffect(() => {
    const context = canvas.current?.getContext('2d');
    if (!context) return;
    if (content === 'cpu') paintDialFace(context, { kind: 'cpu', percent: illustrativeCpuPercent });
    else if (clock) paintDialFace(context, { kind: content, time });
    else if (profile.dial_functions?.length) {
      // A run starts at the first function, so the preview shows it.
      paintDialFace(context, { kind: 'function', name: profile.dial_functions[0].name, index: 0, count: profile.dial_functions.length });
    } else paintDialFace(context, { kind: 'profile', name: profile.name });
  }, [clock, content, profile, time]);
  return <canvas ref={canvas} width={dialFaceSide} height={dialFaceSide} className="dial-face" aria-hidden="true" />;
}
