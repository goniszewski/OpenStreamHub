import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n-react';
import type { MessageKey } from '../locales/en';
import { DemoKeys } from './DemoKeys';
import { tilePng } from './key-tile';
import type { Tile } from './tictactoe-game';
import { centreKey, keepTwofoldBest, press, readTwofoldBest, scoreMs, slideTo, start, tiles, turnDirection, wonMs } from './twofold-game';
import type { Direction, Twofold } from './twofold-game';
import type { DemoEnd, DemoRuntime } from './demo-runtime';
import type { DemoModule } from './demo-catalog';

const storage = () => globalThis.localStorage ?? { getItem: () => null, setItem: () => {} };
const random = () => Math.random();

const arrows: Record<string, Direction> = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };

// A module whose turns make moves beside the keys: which it is, its name on the page, and where it is.
export interface TurnModule { module: DemoModule; name: MessageKey; pathHex: string }

type TurnNotice = { key: 'demos.twofold.turnsFailed'; error: string } | { key: 'demos.twofold.turnsStopped' };

// Twofold on the 15 keys, with the same board shown here. Presses on the keys, clicks on the board, the Mac's arrow
// keys and (when one is connected) a knob or roller module's turns all play the same game. Only the newest board is
// drawn: moves made while the keys are still drawing are not shown one by one.
export function TwofoldScreen({ runtime, pathHex, turns, onEnd }: { runtime: DemoRuntime; pathHex: string; turns?: TurnModule; onEnd(end: DemoEnd | { reason: 'start-failed'; error: string }): void }) {
  const t = useT();
  const [state, setState] = useState<Twofold>(() => start(random, readTwofoldBest(storage())));
  const [ready, setReady] = useState(false);
  const [turnNotice, setTurnNotice] = useState<TurnNotice | null>(null);
  const [turnsJoined, setTurnsJoined] = useState(false);
  const current = useRef(state);
  const ended = useRef(false);
  const readyRef = useRef(false);
  const overlayTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pending = useRef<Tile[] | null>(null);
  const drawing = useRef(false);

  const draw = (next: Tile[]) => {
    pending.current = next;
    if (drawing.current) return;
    drawing.current = true;
    const loop = () => {
      const latest = pending.current;
      pending.current = null;
      if (!latest) { drawing.current = false; return; }
      void runtime.render(latest).catch(() => {}).then(loop);
    };
    loop();
  };

  const apply = (next: Twofold) => {
    if (next === current.current) return;
    current.current = next;
    setState(next);
    if (next.game.score > readTwofoldBest(storage())) keepTwofoldBest(storage(), next.game.score);
    if (overlayTimer.current) { clearTimeout(overlayTimer.current); overlayTimer.current = null; }
    if (next.overlay !== 'none') {
      overlayTimer.current = setTimeout(() => {
        overlayTimer.current = null;
        if (current.current === next) apply({ ...next, overlay: 'none' });
      }, next.overlay === 'score' ? scoreMs : wonMs);
    }
    draw(tiles(next));
  };

  const onKey = (key: number) => apply(press(current.current, key, random));
  const onMove = (direction: Direction) => apply(slideTo(current.current, direction, random));
  // The second module's press works as the centre key.
  const onTurnPress = () => onKey(centreKey);
  const onTurn = (control: number, turn: string) => {
    const direction = turnDirection(control, turn);
    if (direction) onMove(direction);
  };

  useEffect(() => {
    let live = true;
    const finish = (end: Parameters<typeof onEnd>[0]) => { if (!ended.current) { ended.current = true; onEnd(end); } };
    const second = turns ? runtime.another() : null;
    runtime.start(pathHex, key => { if (live) onKey(key); }, end => finish(end)).then(
      () => {
        if (!live) return;
        readyRef.current = true;
        setReady(true);
        draw(tiles(current.current));
        if (!second || !turns) return;
        // The second module is a convenience: the game goes on on the keys when it cannot join or stops answering.
        second.start(turns.pathHex, () => { if (live) onTurnPress(); }, end => {
          if (live && end.reason !== 'user') { setTurnsJoined(false); setTurnNotice({ key: 'demos.twofold.turnsStopped' }); }
        }, (control, turn) => { if (live) onTurn(control, turn); }, false).then(
          () => { if (live) setTurnsJoined(true); },
          reason => { if (live) setTurnNotice({ key: 'demos.twofold.turnsFailed', error: reason instanceof Error ? reason.message : String(reason) }); },
        );
      },
      reason => { if (live) finish({ reason: 'start-failed', error: reason instanceof Error ? reason.message : String(reason) }); },
    );
    const onKeyboard = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { void runtime.end('user'); return; }
      const direction = arrows[event.key];
      if (!direction || !live || !readyRef.current) return;
      event.preventDefault();
      onMove(direction);
    };
    window.addEventListener('keydown', onKeyboard);
    return () => {
      live = false;
      if (overlayTimer.current) clearTimeout(overlayTimer.current);
      window.removeEventListener('keydown', onKeyboard);
      void second?.end('user');
      void runtime.end('user');
    };
  // The demo starts once for its modules; the handlers read the latest game through the ref.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime, pathHex, turns?.pathHex]);

  const { game } = state;
  const headline = game.over ? t('demos.twofold.over', { score: game.score })
    : state.overlay === 'won' ? t('demos.twofold.won')
    : t('demos.twofold.playing');
  return <section className="panel demo-play" aria-label={t('demos.twofold.name')}>
    <h2>{t('demos.twofold.name')}</h2>
    {!ready && <p className="fine-print" role="status">{t('demos.starting')}</p>}
    <p role="status" className="demo-status">{headline}</p>
    <DemoKeys images={tiles(state).map(tile => tilePng(tile))} label={key => t('demos.key', { number: key + 1 })} onPress={ready ? onKey : undefined} />
    <p className="fine-print">{t('demos.twofold.score', { score: game.score, best: game.best, moves: game.moves })}</p>
    {turns && turnsJoined && <p className="fine-print">{t(turns.module === 'm003k' ? 'demos.twofold.turnsKnobs' : 'demos.twofold.turnsRollers', { module: t(turns.name) })}</p>}
    {turns && turnNotice && <p className="fine-print" role="alert">{t(turnNotice.key, { module: t(turns.name), error: 'error' in turnNotice ? turnNotice.error : '' })}</p>}
    <div className="demo-actions">
      <button type="button" className="button" disabled={!ready} onClick={() => apply(start(random, current.current.game.best))}>{t('demos.newGame')}</button>
      <button type="button" className="button" onClick={() => void runtime.end('user')}>{t('demos.exit')}</button>
    </div>
    <p className="fine-print">{t('demos.twofold.hint')}</p>
  </section>;
}
