import { TriangleAlert } from 'lucide-react';
import type { ServiceRun } from './service-runner';
import { modules } from './ui-model';

// The automatic sessions that run actions now: started by the session manager, executing, not stopped.
export function automaticExecutingRuns(runs: Record<string, ServiceRun>): ServiceRun[] {
  return Object.values(runs).filter(run => run.session?.autoStart !== undefined && run.execute && run.session.phase !== 'stopped');
}

export const autoActionsText = (runs: ServiceRun[]) => {
  const names = [...new Set(runs.map(run => modules.find(entry => entry.id === run.session!.module)?.shortName ?? run.session!.module))];
  return `Actions run automatically on ${names.join(', ')}, with no confirmation.`;
};

// Persistent and non-blocking while any automatic session executes. One click turns "Run actions automatically" off,
// which makes the backend stop those processes at once and restart them as dry runs.
export function AutoActionsIndicator({ runs, busy = false, onStop }: { runs: Record<string, ServiceRun>; busy?: boolean; onStop(): void }) {
  const executing = automaticExecutingRuns(runs);
  if (!executing.length) return null;
  return <div className="alert error auto-actions-indicator" role="status">
    <TriangleAlert size={18} aria-hidden="true" /><span>{autoActionsText(executing)}</span>
    <button type="button" className="button compact danger" disabled={busy} onClick={onStop}>Stop automatic actions</button>
  </div>;
}
