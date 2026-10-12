import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n-react';
import { DemoKeys } from './DemoKeys';
import { tilePng } from './key-tile';
import { keys, newGame, press, start, status, tiles } from './tictactoe-game';
import type { Game } from './tictactoe-game';
import type { DemoEnd, DemoRuntime } from './demo-runtime';

// How long each half of a blink lasts.
export const blinkMs = 450;

// Tic-tac-toe on the module's keys, with the same game shown here. Presses on the keys and clicks on the board play the
// same game; the page draws only what changed on the keys.
export function TicTacToeScreen({ runtime, pathHex, onEnd }: { runtime: DemoRuntime; pathHex: string; onEnd(end: DemoEnd | { reason: 'start-failed'; error: string }): void }) {
  const t = useT();
  const [game, setGame] = useState<Game>(() => start(newGame()));
  const [ready, setReady] = useState(false);
  const [lit, setLit] = useState(true);
  const current = useRef(game);
  const litRef = useRef(true);
  const ended = useRef(false);

  // What the module shows, and the same pictures here.
  const show = (next: Game, on: boolean) => void runtime.render(tiles(next, on)).catch(() => {});

  const play = (key: number) => {
    const next = press(current.current, key);
    if (next === current.current) return;
    current.current = next;
    setGame(next);
    if (next.exit) { void runtime.end('user'); return; }
    litRef.current = true;
    setLit(true);
    show(next, true);
  };

  useEffect(() => {
    let live = true;
    const finish = (end: Parameters<typeof onEnd>[0]) => { if (!ended.current) { ended.current = true; onEnd(end); } };
    runtime.start(pathHex, key => { if (live) play(key); }, end => finish(end)).then(
      () => { if (!live) return; setReady(true); show(current.current, litRef.current); },
      reason => { if (live) finish({ reason: 'start-failed', error: reason instanceof Error ? reason.message : String(reason) }); },
    );
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') void runtime.end('user'); };
    window.addEventListener('keydown', onKey);
    return () => { live = false; window.removeEventListener('keydown', onKey); void runtime.end('user'); };
  // The demo starts once for this module; `play` reads the latest game through the ref.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime, pathHex]);

  // A finished game with a winner blinks its line on the module and here.
  const winning = status(game).kind === 'won';
  useEffect(() => {
    if (!winning || !ready) return;
    const timer = setInterval(() => {
      litRef.current = !litRef.current;
      setLit(litRef.current);
      show(current.current, litRef.current);
    }, blinkMs);
    return () => clearInterval(timer);
  // The blink restarts with each game that is won.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [winning, ready, game.board]);

  const state = status(game);
  const mark = state.kind === 'draw' ? '' : state.mark;
  const headline = state.kind === 'draw' ? t('demos.ticTacToe.draw')
    : state.kind === 'turn' ? t(state.yours ? 'demos.ticTacToe.turnYours' : 'demos.ticTacToe.turn', { mark })
    : t(state.yours === true ? 'demos.ticTacToe.wonYours' : state.yours === false ? 'demos.ticTacToe.wonTheirs' : 'demos.ticTacToe.won', { mark });
  return <section className="panel demo-play" aria-label={t('demos.ticTacToe.name')}>
    <h2>{t('demos.ticTacToe.name')}</h2>
    {!ready && <p className="fine-print" role="status">{t('demos.starting')}</p>}
    <p role="status" className="demo-status">{headline}</p>
    <DemoKeys images={tiles(game, lit).map(tile => tilePng(tile))} label={key => key < 15 ? t('demos.key', { number: key + 1 }) : ''} onPress={ready ? play : undefined} />
    <p className="fine-print">{t('demos.ticTacToe.score', { x: game.score.X, o: game.score.O, draws: game.score.draws })}</p>
    <div className="demo-actions">
      <button type="button" className="button" disabled={!ready} onClick={() => play(keys.newGame)}>{t('demos.newGame')}</button>
      <button type="button" className="button" onClick={() => void runtime.end('user')}>{t('demos.exit')}</button>
    </div>
    <p className="fine-print">{t('demos.exitHint')}</p>
  </section>;
}

export const demoTilePng = (tile: unknown) => tilePng(tile as Parameters<typeof tilePng>[0]);
