import { ArrowRight } from 'lucide-react';
import type { Invoke } from './profile-session';

// The app's own icon, draggable onto the list of a System Settings privacy pane. The arrow points at the icon
// so it is clear what to pick up; the native drag starts on mouse down.
export function PermissionDragTile({ invoke, onError }: { invoke: Invoke; onError?(message: string): void }) {
  const start = () => { void invoke('start_permission_drag').catch(reason => onError?.(String(reason))); };
  return <div className="permission-drag" role="group" aria-label="Drag OpenStreamHub into System Settings">
    <span className="permission-drag-hint">Drag this icon into the list in System Settings, then switch it on.</span>
    <ArrowRight size={18} className="permission-drag-arrow" aria-hidden />
    <button className="permission-drag-tile" onMouseDown={start}><img src="/app-icon.png" alt="" width={32} height={32} />OpenStreamHub</button>
  </div>;
}
