import { useState } from 'react';
import { Gamepad2 } from 'lucide-react';
import { demoAvailable, demoMissing, demoModuleName, demos, modulePresent } from './demos/demo-catalog';
import type { DemoEnd, DemoRuntime } from './demos/demo-runtime';
import { SpeedTestScreen } from './demos/SpeedTestScreen';
import { TicTacToeScreen } from './demos/TicTacToeScreen';
import { WhackScreen } from './demos/WhackScreen';
import { useT } from './i18n-react';

type Ended = DemoEnd | { reason: 'start-failed'; error: string };

// The gallery of demos. Cards are read-only in the browser preview; playing one needs the desktop app and a connected
// module. A running demo takes the page over until it ends.
export function DemosPage({ desktop, devices, runtime }: { desktop: boolean; devices: readonly { module: string; path_hex?: string }[]; runtime?: DemoRuntime }) {
  const t = useT();
  const [playing, setPlaying] = useState<{ demo: string; pathHex: string } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const ended = (end: Ended) => {
    setPlaying(null);
    setNotice(end.reason === 'user' ? null : end.reason === 'start-failed' ? t('demos.startFailed', { error: end.error })
      : end.reason === 'module-stopped' ? t('demos.ended.module-stopped') : t('demos.ended.failed', { error: end.error ?? '' }));
  };
  if (playing && runtime) return <section className="page-content demos-page">
    <div className="page-title"><div><h1>{t('demos.title')}</h1><p>{t('demos.paused')}</p></div></div>
    {playing.demo === 'display-speed'
      ? <SpeedTestScreen runtime={runtime} pathHex={playing.pathHex} onEnd={ended} />
      : playing.demo === 'whack-a-mole'
        ? <WhackScreen runtime={runtime} pathHex={playing.pathHex} onEnd={ended} />
        : <TicTacToeScreen runtime={runtime} pathHex={playing.pathHex} onEnd={ended} />}
  </section>;
  return <section className="page-content demos-page">
    <div className="page-title"><div><h1>{t('demos.title')}</h1><p>{t('demos.intro')}</p></div></div>
    <p className="fine-print">{t('demos.paused')}</p>
    {!desktop && <p className="fine-print">{t('demos.preview')}</p>}
    {notice && <p className="fine-print" role="alert">{notice}</p>}
    <div className="feature-grid">
      {demos.map(demo => {
        const names = demo.modules.map(module => t(demoModuleName[module])).join(', ');
        const missing = demoMissing(demo, devices).map(module => t(demoModuleName[module])).join(', ');
        const device = devices.find(entry => entry.module === demo.modules[0]);
        const available = desktop && demoAvailable(demo, devices);
        const playable = available && runtime !== undefined && device?.path_hex !== undefined;
        return <section key={demo.id} className="panel demo-card" aria-labelledby={`demo-${demo.id}`}>
          <h2 id={`demo-${demo.id}`}><Gamepad2 size={18} aria-hidden="true" /> {t(demo.name)}</h2>
          <p>{t(demo.description)}</p>
          <ul className="demo-tags" aria-label={t('demos.needsAll', { modules: names })}>
            {demo.modules.map(module => {
              const present = modulePresent(module, devices);
              return <li key={module} className={`demo-tag${present ? ' present' : ''}`}>{t(demoModuleName[module])}<span className="sr-only"> ({t(present ? 'demos.tag.connected' : 'demos.tag.missing')})</span></li>;
            })}
          </ul>
          <p className="fine-print">{missing ? t('demos.needsMissingSome', { modules: names, missing }) : t('demos.needsAll', { modules: names })}</p>
          <button className="button" disabled={!playable} onClick={() => { setNotice(null); setPlaying({ demo: demo.id, pathHex: device!.path_hex! }); }}>{t('demos.play')}</button>
        </section>;
      })}
    </div>
  </section>;
}
