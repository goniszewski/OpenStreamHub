import { ArrowLeft, ArrowLeftRight, Camera, Folder, Globe, Keyboard, ListOrdered, Monitor, Play, Timer, Type, Undo2, Volume2, VolumeX } from 'lucide-react';
import type { BindingAction, KeypadPage, KeypadSlot } from './contracts';
import { actionLabel } from './ui-model';

export function ActionIcon({ action }: { action?: BindingAction }) {
  const Icon = action?.type === 'toggle_mute' ? VolumeX : action?.type === 'launch_app' ? Monitor :
    action?.type === 'volume' || action?.type === 'set_volume' || action?.type === 'volume_from_position' ? Volume2 :
    action?.type === 'open_url' ? Globe : action?.type === 'media' ? Play : action?.type === 'text' ? Type :
    action?.type === 'multi_action' ? ListOrdered : action?.type === 'profile' ? ArrowLeftRight : action?.type === 'timer' ? Timer : action?.type === 'system' ? (action.command === 'screenshot' ? Camera : Undo2) : Keyboard;
  return <Icon aria-hidden="true" />;
}

// The same content is used in the editor and the physical module illustration.
export function KeyDisplayContent({ slot, pages, depth, back }: { slot: KeypadSlot; pages: KeypadPage[]; depth: number; back: boolean }) {
  if (back) return <><ArrowLeft aria-hidden="true" /><span className="display-depth">L{depth}</span><span className="display-label">Back</span></>;
  if (!slot) return <span className="display-empty" aria-hidden="true">·</span>;
  if (slot.type === 'folder') return <><Folder aria-hidden="true" /><span className="display-label">{pages.find(page => page.id === slot.page_id)?.name ?? slot.page_id}</span></>;
  return <><ActionIcon action={slot.bindings[0]?.action} /><span className="display-label">{slot.label}</span></>;
}

export function EncoderDisplayContent({ action }: { action?: BindingAction }) {
  const label = action?.type === 'volume' ? `Volume ${action.delta > 0 ? '+' : ''}${action.delta}` :
    action?.type === 'toggle_mute' ? 'Mute' : action?.type === 'set_volume' ? `Volume ${action.percent}%` : action ? actionLabel(action) : '';
  return <span className="encoder-display" aria-hidden="true">
    {action ? <><ActionIcon action={action} /><span className="display-label">{label}</span></> : <><Monitor /><span className="display-label">No binding</span></>}
  </span>;
}
