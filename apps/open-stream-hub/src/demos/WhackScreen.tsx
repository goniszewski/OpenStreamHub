import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n-react';
import { DemoKeys } from './DemoKeys';
import { demoTilePng } from './TicTacToeScreen';
import { appear, clearFlash, clearLate, escaped, flashMs, gapMs, graceMs, hit, keepBest, light, moleDuration, newReaction, newWhack, randomWait, reactionDone, reactionPress, reactionRounds, reactionTiles, readBest, roundMs, summarise, whackTiles } from './whack-game';
import type { Reaction, Whack } from './whack-game';
import type { DemoEnd, DemoRuntime } from './demo-runtime';

type Mode = 'whack' | 'reaction';
type Phase = 'ready' | 'playing' | 'over';

const storage = () => globalThis.localStorage ?? { getItem: () => null, setItem: () => {} };

// Whack-a-mole and the reaction test on the 15 keys. Both draw only what changed, as the speed test showed the module takes it.
export function WhackScreen({ runtime, pathHex, onEnd }: { runtime: DemoRuntime; pathHex: string; onEnd(end: DemoEnd | { reason: 'start-failed'; error: string }): void }) {
  const t = useT();
  const [ready, setReady] = useState(false);
  const [mode, setMode] = useState<Mode>('whack');
  const [phase, setPhase] = useState<Phase>('ready');
  const [whack, setWhack] = useState<Whack>(newWhack);
  const [reaction, setReaction] = useState<Reaction>(newReaction);
  const [secondsLeft, setSecondsLeft] = useState(roundMs / 1000);
  const [best, setBest] = useState(() => readBest(storage()));
  const [newBest, setNewBest] = useState(false);
  const whackRef = useRef(whack);
  const reactionRef = useRef(reaction);
  const phaseRef = useRef<Phase>('ready');
  const modeRef = useRef<Mode>('whack');
  const startedAt = useRef(0);
  const litAt = useRef<number | null>(null);
  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  const moleTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const ended = useRef(false);

  const later = (callback: () => void, ms: number) => {
    const handle = setTimeout(() => { timers.current.delete(handle); callback(); }, ms);
    timers.current.add(handle);
    return handle;
  };
  const cancelAll = () => { timers.current.forEach(clearTimeout); timers.current.clear(); moleTimer.current = null; };
  const setPhaseBoth = (next: Phase) => { phaseRef.current = next; setPhase(next); };
  const drawn = (tiles: unknown[]) => void runtime.render(tiles).catch(() => {});

  const showWhack = (next: Whack) => { whackRef.current = next; setWhack(next); drawn(whackTiles(next)); };
  const showReaction = (next: Reaction) => { reactionRef.current = next; setReaction(next); drawn(reactionTiles(next)); };

  // After a hit or a miss the mark stays a moment, then goes.
  const clearLater = (flash: Whack['flash']) => later(() => { if (whackRef.current.flash === flash) showWhack(clearFlash(whackRef.current)); }, flashMs);

  const spawn = () => {
    if (phaseRef.current !== 'playing') return;
    const next = appear(whackRef.current);
    whackRef.current = next;
    setWhack(next);
    // The mole's time starts once its picture is on the module, so a slow draw does not eat into it.
    void runtime.render(whackTiles(next)).catch(() => {}).then(() => {
      if (phaseRef.current !== 'playing' || whackRef.current !== next) return;
      moleTimer.current = later(() => {
        moleTimer.current = null;
        const missed = escaped(whackRef.current);
        showWhack(missed);
        clearLater(missed.flash);
        later(() => { if (whackRef.current.late === missed.late) whackRef.current = clearLate(whackRef.current); }, graceMs);
        later(spawn, gapMs);
      }, moleDuration(performance.now() - startedAt.current));
    });
  };

  const finishWhack = () => {
    cancelAll();
    const final = { ...whackRef.current, mole: null, flash: null };
    showWhack(final);
    const before = readBest(storage());
    setBest(keepBest(storage(), final.score));
    setNewBest(final.score > before);
    setPhaseBoth('over');
  };

  const startWhack = () => {
    cancelAll();
    setNewBest(false);
    startedAt.current = performance.now();
    setSecondsLeft(roundMs / 1000);
    showWhack(newWhack());
    setPhaseBoth('playing');
    later(spawn, gapMs * 2);
    later(finishWhack, roundMs);
    const tick = () => { setSecondsLeft(Math.max(0, Math.ceil((roundMs - (performance.now() - startedAt.current)) / 1000))); if (phaseRef.current === 'playing') later(tick, 250); };
    later(tick, 250);
  };

  const waitForLight = () => later(() => {
    litAt.current = null;
    const next = light(reactionRef.current);
    reactionRef.current = next;
    setReaction(next);
    // The clock starts once the key has been drawn, so the time is the person's, not the drawing's.
    void runtime.render(reactionTiles(next)).catch(() => {}).then(() => { if (reactionRef.current === next) litAt.current = performance.now(); });
  }, randomWait());

  const startReaction = () => {
    cancelAll();
    setNewBest(false);
    litAt.current = null;
    showReaction(newReaction());
    setPhaseBoth('playing');
    waitForLight();
  };

  const onKey = (key: number) => {
    if (phaseRef.current !== 'playing') return;
    if (modeRef.current === 'whack') {
      const before = whackRef.current;
      const next = hit(before, key);
      showWhack(next);
      if (next.mole === null && before.mole !== null) {
        if (moleTimer.current) { clearTimeout(moleTimer.current); timers.current.delete(moleTimer.current); moleTimer.current = null; }
        later(spawn, gapMs);
      }
      clearLater(next.flash);
      return;
    }
    const current = reactionRef.current;
    if (current.lit !== null && litAt.current === null) return;
    const next = reactionPress(current, key, performance.now() - (litAt.current ?? 0));
    if (next === current) return;
    litAt.current = null;
    showReaction(next);
    if (reactionDone(next)) { setPhaseBoth('over'); cancelAll(); } else if (next.lit === null && current.lit !== null) waitForLight();
  };

  const choose = (next: Mode) => {
    if (phaseRef.current === 'playing') return;
    modeRef.current = next;
    setMode(next);
    setPhaseBoth('ready');
    showWhack(newWhack());
    showReaction(newReaction());
    setNewBest(false);
  };

  useEffect(() => {
    let live = true;
    const finish = (end: Parameters<typeof onEnd>[0]) => { if (!ended.current) { ended.current = true; onEnd(end); } };
    runtime.start(pathHex, key => { if (live) onKey(key); }, end => finish(end)).then(
      () => { if (live) { setReady(true); drawn(whackTiles(newWhack())); } },
      reason => { if (live) finish({ reason: 'start-failed', error: reason instanceof Error ? reason.message : String(reason) }); },
    );
    const onEsc = (event: KeyboardEvent) => { if (event.key === 'Escape') void runtime.end('user'); };
    window.addEventListener('keydown', onEsc);
    return () => { live = false; cancelAll(); window.removeEventListener('keydown', onEsc); void runtime.end('user'); };
  // The demo starts once for this module; the handlers read the latest state through refs.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime, pathHex]);

  const times = reaction.times;
  const summary = summarise(times);
  const headline = phase === 'ready' ? t(mode === 'whack' ? 'demos.whack.ready' : 'demos.whack.reactionReady')
    : mode === 'whack'
      ? phase === 'playing' ? t('demos.whack.playing', { seconds: secondsLeft }) : t('demos.whack.over', { score: whack.score, hits: whack.hits, misses: whack.misses })
      : phase === 'over' ? t('demos.whack.reactionOver') : t(reaction.lit === null ? 'demos.whack.reactionWait' : 'demos.whack.reactionGo', { round: times.length + 1, total: reactionRounds });
  const tiles = mode === 'whack' ? whackTiles(whack) : reactionTiles(reaction);
  return <section className="panel demo-play" aria-label={t('demos.whack.name')}>
    <h2>{t('demos.whack.name')}</h2>
    {!ready && <p className="fine-print" role="status">{t('demos.starting')}</p>}
    <div className="demo-actions" role="group" aria-label={t('demos.whack.mode')}>
      {(['whack', 'reaction'] as const).map(entry => <button key={entry} type="button" className="button" aria-pressed={mode === entry} disabled={phase === 'playing'} onClick={() => choose(entry)}>{t(`demos.whack.mode.${entry}` as const)}</button>)}
    </div>
    <p role="status" className="demo-status">{headline}</p>
    <DemoKeys images={tiles.map(tile => demoTilePng(tile))} label={key => t('demos.key', { number: key + 1 })} onPress={ready && phase === 'playing' ? onKey : undefined} />
    {mode === 'whack'
      ? <p className="fine-print">{t('demos.whack.score', { score: whack.score, best })}{newBest && ` ${t('demos.whack.newBest')}`}</p>
      : <p className="fine-print">{times.length > 0 && t('demos.whack.times', { times: times.join(', ') })}{summary && ` ${t('demos.whack.summary', { ...summary })}`}{reaction.early > 0 && ` ${t('demos.whack.early', { count: reaction.early })}`}</p>}
    <div className="demo-actions">
      <button type="button" className="button" disabled={!ready || phase === 'playing'} onClick={mode === 'whack' ? startWhack : startReaction}>{t(phase === 'over' ? 'demos.whack.again' : 'demos.whack.start')}</button>
      <button type="button" className="button" onClick={() => void runtime.end('user')}>{t('demos.exit')}</button>
    </div>
    <p className="fine-print">{t('demos.whack.hint')}</p>
  </section>;
}
