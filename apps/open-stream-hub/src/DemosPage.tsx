import { useState } from 'react';
import { Gamepad2 } from 'lucide-react';
import { demoAvailable, demoModuleName, demos } from './demos/demo-catalog';
import type { DemoEnd, DemoRuntime } from './demos/demo-runtime';
import { TicTacToeScreen } from './demos/TicTacToeScreen';
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
    <TicTacToeScreen runtime={runtime} pathHex={playing.pathHex} onEnd={ended} />
  </section>;
  return <section className="page-content demos-page">
    <div className="page-title"><div><h1>{t('demos.title')}</h1><p>{t('demos.intro')}</p></div></div>
    <p className="fine-print">{t('demos.paused')}</p>
    {!desktop && <p className="fine-print">{t('demos.preview')}</p>}
    {notice && <p className="fine-print" role="alert">{notice}</p>}
    <div className="feature-grid">
      {demos.map(demo => {
        const module = t(demoModuleName[demo.module]);
        const device = devices.find(entry => entry.module === demo.module);
        const available = desktop && demoAvailable(demo, devices);
        const playable = available && runtime !== undefined && device?.path_hex !== undefined;
        return <section key={demo.id} className="panel demo-card" aria-labelledby={`demo-${demo.id}`}>
          <h2 id={`demo-${demo.id}`}><Gamepad2 size={18} aria-hidden="true" /> {t(demo.name)}</h2>
          <p>{t(demo.description)}</p>
          <p className="fine-print">{demoAvailable(demo, devices) ? t('demos.needs', { module }) : t('demos.needsMissing', { module })}</p>
          <button className="button" disabled={!playable} onClick={() => { setNotice(null); setPlaying({ demo: demo.id, pathHex: device!.path_hex! }); }}>{t('demos.play')}</button>
        </section>;
      })}
    </div>
  </section>;
}
