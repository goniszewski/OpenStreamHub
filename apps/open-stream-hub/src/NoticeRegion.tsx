import { useSyncExternalStore } from 'react';
import { CheckCircle2, CircleHelp, Cable, PauseCircle, X } from 'lucide-react';
import { noticeFocusAfterDismiss } from './notices';
import type { NoticeKind, NoticeStore } from './notices';
import type { Notice } from './notices';

const icons: Record<NoticeKind, typeof X> = { connected: CheckCircle2, reconnected: CheckCircle2, replaced: Cable, disconnected: Cable, paused: PauseCircle, stopped: CircleHelp };

// Where focus goes when the last notice is dismissed while it held focus.
export const noticeFallbackFocusId = 'main-content';

// Focuses the element `find` returns once it exists, trying for a few frames
// (for example after a page change has rendered). Gives up quietly.
export function focusWhenRendered(find: () => { focus(): void } | null, schedule: (callback: () => void) => void = callback => requestAnimationFrame(callback), frames = 10) {
  const attempt = (left: number) => {
    const element = find();
    if (element) element.focus();
    else if (left > 0) schedule(() => attempt(left - 1));
  };
  schedule(() => attempt(frames));
}

// Polite live region that is always mounted so screen readers announce what appears in it.
// It never takes focus on its own; dismissing a focused notice hands focus to the next one.
export function NoticeRegion({ store, onReview }: { store: NoticeStore; onReview(): void }) {
  const { notices } = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  return <NoticeList notices={notices} onDismiss={id => store.dismiss(id)} onPause={id => store.pause(id)} onResume={id => store.resume(id)} onReview={onReview} />;
}

export function NoticeList({ notices, onDismiss, onPause, onResume, onReview }: {
  notices: Notice[]; onDismiss(id: number): void; onPause(id: number): void; onResume(id: number): void; onReview(): void;
}) {
  const dismiss = (id: number, toast: HTMLElement | null) => {
    if (toast?.contains(document.activeElement)) {
      const next = noticeFocusAfterDismiss(notices.map(notice => notice.id), id);
      const target = next === null ? document.getElementById(noticeFallbackFocusId)
        : toast.parentElement?.querySelector<HTMLElement>(`[data-notice-id="${next}"] .toast-close`) ?? null;
      target?.focus({ preventScroll: true });
    }
    onDismiss(id);
  };
  return <div className="toast-region" role="status" aria-live="polite" aria-relevant="additions text">
    {notices.map(notice => {
      const Icon = icons[notice.kind];
      // Hover and focus each hold the notice open; it resumes only when neither does.
      return <div key={notice.id} className={`toast ${notice.kind}`} data-notice-id={notice.id}
        onMouseEnter={() => onPause(notice.id)} onMouseLeave={event => { if (!event.currentTarget.contains(document.activeElement)) onResume(notice.id); }}
        onFocus={() => onPause(notice.id)} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null) && !event.currentTarget.matches(':hover')) onResume(notice.id); }}>
        <Icon size={17} aria-hidden="true" /><span className="toast-text">{notice.text}</span>
        {notice.action === 'review' && <button className="button compact" onClick={onReview}>Review</button>}
        <button className="toast-close" aria-label={`Dismiss notice: ${notice.text}`} onClick={event => dismiss(notice.id, event.currentTarget.closest<HTMLElement>('.toast'))}><X size={15} /></button>
      </div>;
    })}
  </div>;
}
