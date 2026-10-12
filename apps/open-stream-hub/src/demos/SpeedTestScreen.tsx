import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n-react';
import { DemoKeys } from './DemoKeys';
import { demoTilePng } from './TicTacToeScreen';
import { describeResults, previewTiles, quantiles, safeFrameRate } from './speed-test';
import type { Pass, PassResult, Progress } from './speed-test';
import type { DemoEnd, DemoRuntime } from './demo-runtime';
import type { Tile } from './tictactoe-game';

// Seconds each pass runs. The presses of the latency check are few and counted, not timed.
export const passSeconds = 30;
const presses = 5;
// The keys the latency check lights, in turn.
const latencyKeys = [0, 7, 14, 4, 10];

type Step = 'ready' | 'starting' | 'grid' | 'single' | 'latency' | 'done';

const lit = (key: number): Tile[] => Array.from({ length: 15 }, (_, index) => index === key ? { tone: 'fill', colour: '#ffffff' } : { tone: 'blank' });

// The display speed test: how fast the keys take pictures, and how quickly a key redraws after it is pressed.
export function SpeedTestScreen({ runtime, pathHex, onEnd }: { runtime: DemoRuntime; pathHex: string; onEnd(end: DemoEnd | { reason: 'start-failed'; error: string }): void }) {
  const t = useT();
  const [step, setStep] = useState<Step>('ready');
  const [shown, setShown] = useState<Tile[]>(() => previewTiles('grid', 0));
  const [progress, setProgress] = useState<Progress | null>(null);
  const [results, setResults] = useState<PassResult[]>([]);
  const [latency, setLatency] = useState<number[]>([]);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ended = useRef(false);
  const cancelled = useRef(false);
  const waiting = useRef<((key: number) => void) | null>(null);

  const finish = (end: Parameters<typeof onEnd>[0]) => { if (!ended.current) { ended.current = true; onEnd(end); } };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') leave(); };
    window.addEventListener('keydown', onKey);
    return () => { window.removeEventListener('keydown', onKey); cancelled.current = true; void runtime.cancelSpeed().catch(() => {}); void runtime.end('user'); };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime]);

  // Leaving before the test started needs no demo to end: the module was never taken.
  const leave = () => {
    cancelled.current = true;
    if (runtime.getPhase() === 'idle') { finish({ reason: 'user' }); return; }
    void runtime.cancelSpeed().catch(() => {});
    void runtime.end('user');
  };

  async function pass(kind: Pass, name: Step): Promise<PassResult> {
    setStep(name);
    const result = await runtime.speedPass<PassResult>(kind, passSeconds, value => {
      const next = value as Progress;
      setProgress(next);
      setShown(previewTiles(kind, next.writes));
    });
    setResults(current => [...current, result]);
    return result;
  }

  async function run() {
    setError(null); setResults([]); setLatency([]); setProgress(null); cancelled.current = false;
    setStep('starting');
    try {
      if (runtime.getPhase() === 'idle') await runtime.start(pathHex, key => waiting.current?.(key), end => { if (end.reason !== 'user') setError(end.error ?? ''); finish(end); });
      await runtime.render(previewTiles('grid', 0));
      await pass('grid', 'grid');
      if (cancelled.current) return setStep('done');
      await pass('single', 'single');
      if (cancelled.current) return setStep('done');
      await measureLatency();
      setStep('done');
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      if (!ended.current && !cancelled.current) { setError(message); setStep('done'); }
    }
  }

  async function measureLatency() {
    setStep('latency');
    const dark = demoTilePng({ tone: 'blank' })!;
    const bright = demoTilePng({ tone: 'fill', colour: '#ffffff' })!;
    const samples: number[] = [];
    for (const key of latencyKeys.slice(0, presses)) {
      if (cancelled.current) return;
      setShown(lit(key));
      await runtime.render(Array.from({ length: 15 }, (_, index) => index === key ? bright : dark));
      await new Promise<void>(resolve => { waiting.current = pressed => { if (pressed === key) { waiting.current = null; resolve(); } }; });
      // The press is in: time the one write that darkens the key.
      const started = performance.now();
      await runtime.render(Array.from({ length: 15 }, () => dark));
      samples.push(performance.now() - started);
      setLatency([...samples]);
      setShown(previewTiles('grid', 0));
    }
  }

  const grid = results.find(result => result.pass === 'grid');
  const rate = safeFrameRate(grid);
  const text = describeResults(results, latency, new Date().toISOString().slice(0, 10));
  const copy = () => { void navigator.clipboard?.writeText(text).then(() => setCopied(true)).catch(() => {}); };
  const running = step === 'starting' || step === 'grid' || step === 'single' || step === 'latency';
  return <section className="panel demo-play" aria-label={t('demos.speed.name')}>
    <h2>{t('demos.speed.name')}</h2>
    <p className="demo-status" role="status">{t(`demos.speed.step.${step}` as const)}{step === 'latency' && ` ${latency.length}/${presses}`}</p>
    {progress && (step === 'grid' || step === 'single') && <p className="fine-print">{t('demos.speed.progress', { seconds: Math.floor(progress.elapsed_ms / 1000), total: passSeconds, writes: progress.writes })}</p>}
    <DemoKeys images={shown.map(tile => demoTilePng(tile))} label={key => t('demos.key', { number: key + 1 })} />
    {error && <p className="fine-print" role="alert">{error}</p>}
    {results.length > 0 && <pre className="speed-results">{text}</pre>}
    {rate !== null && <p className="fine-print">{t('demos.speed.rate', { fps: rate })}</p>}
    <div className="demo-actions">
      {!running && <button type="button" className="button" onClick={() => void run()}>{step === 'done' ? t('demos.speed.again') : t('demos.speed.start')}</button>}
      {step === 'done' && results.length > 0 && <button type="button" className="button" onClick={copy}>{copied ? t('demos.speed.copied') : t('demos.speed.copy')}</button>}
      {running && <button type="button" className="button" onClick={() => { cancelled.current = true; void runtime.cancelSpeed().catch(() => {}); waiting.current?.(-1); }}>{t('demos.speed.stop')}</button>}
      <button type="button" className="button" onClick={leave}>{t('demos.exit')}</button>
    </div>
    <p className="fine-print">{t('demos.speed.hint')}</p>
  </section>;
}
