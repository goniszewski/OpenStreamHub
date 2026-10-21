import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowLeftRight, Camera, Clapperboard, Folder, Globe, Keyboard, ListOrdered, Monitor, Play, Timer, Type, Undo2, Volume2, VolumeX } from 'lucide-react';
import type { BindingAction, DialAnimation, DialDisplayContent, KeyArtwork, KeypadPage, KeypadSlot, Profile } from './contracts';
import { effectiveDialDisplay } from './dial-display-model';
import { dialFaceSide, nowDialTime, paintDialFace } from './dial-face';
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

// The same content is used in the editor and the physical module illustration.
export function KeyDisplayContent({ slot, pages, depth, back, artwork }: { slot: KeypadSlot; pages: KeypadPage[]; depth: number; back: boolean; artwork?: KeyArtwork }) {
  if (back) return <><ArrowLeft aria-hidden="true" /><span className="display-depth">L{depth}</span><span className="display-label">Back</span></>;
  if (artwork) return <KeyArtworkPreview artwork={artwork} />;
  if (!slot) return <span className="display-empty" aria-hidden="true">·</span>;
  if (slot.type === 'folder') return <><Folder aria-hidden="true" /><span className="display-label">{pages.find(page => page.id === slot.page_id)?.name ?? slot.page_id}</span></>;
  return <><ActionIcon action={slot.bindings[0]?.action} /><span className="display-label">{slot.label}</span></>;
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
