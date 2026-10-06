import { useSyncExternalStore } from 'react';
import { CheckCircle2, CircleHelp, Cable, PauseCircle, X } from 'lucide-react';
import type { NoticeKind, NoticeStore } from './notices';
import type { Notice } from './notices';

const icons: Record<NoticeKind, typeof X> = { connected: CheckCircle2, reconnected: CheckCircle2, replaced: Cable, disconnected: Cable, paused: PauseCircle, stopped: CircleHelp };

// Polite live region that is always mounted so screen readers announce what appears in it. It never takes focus.
export function NoticeRegion({ store, onReview }: { store: NoticeStore; onReview(): void }) {
  const { notices } = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  return <NoticeList notices={notices} onDismiss={id => store.dismiss(id)} onPause={id => store.pause(id)} onResume={id => store.resume(id)} onReview={onReview} />;
}

export function NoticeList({ notices, onDismiss, onPause, onResume, onReview }: {
  notices: Notice[]; onDismiss(id: number): void; onPause(id: number): void; onResume(id: number): void; onReview(): void;
}) {
  return <div className="toast-region" role="status" aria-live="polite" aria-relevant="additions text">
    {notices.map(notice => {
      const Icon = icons[notice.kind];
      return <div key={notice.id} className={`toast ${notice.kind}`}
        onMouseEnter={() => onPause(notice.id)} onMouseLeave={() => onResume(notice.id)}
        onFocus={() => onPause(notice.id)} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) onResume(notice.id); }}>
        <Icon size={17} aria-hidden="true" /><span className="toast-text">{notice.text}</span>
        {notice.action === 'review' && <button className="button compact" onClick={onReview}>Review</button>}
        <button className="toast-close" aria-label={`Dismiss notice: ${notice.text}`} onClick={() => onDismiss(notice.id)}><X size={15} /></button>
      </div>;
    })}
  </div>;
}
