import { Gamepad2 } from 'lucide-react';
import { demoAvailable, demoModuleName, demos } from './demos/demo-catalog';
import { useT } from './i18n-react';

// The gallery of demos. Cards are read-only here; playing one needs the desktop app and a connected module.
export function DemosPage({ desktop, devices }: { desktop: boolean; devices: readonly { module: string }[] }) {
  const t = useT();
  return <section className="page-content demos-page">
    <div className="page-title"><div><h1>{t('demos.title')}</h1><p>{t('demos.intro')}</p></div></div>
    <p className="fine-print">{t('demos.paused')}</p>
    {!desktop && <p className="fine-print">{t('demos.preview')}</p>}
    <div className="feature-grid">
      {demos.map(demo => {
        const module = t(demoModuleName[demo.module]);
        const available = desktop && demoAvailable(demo, devices);
        return <section key={demo.id} className="panel demo-card" aria-labelledby={`demo-${demo.id}`}>
          <h2 id={`demo-${demo.id}`}><Gamepad2 size={18} aria-hidden="true" /> {t(demo.name)}</h2>
          <p>{t(demo.description)}</p>
          <p className="fine-print">{demoAvailable(demo, devices) ? t('demos.needs', { module }) : t('demos.needsMissing', { module })}</p>
          <button className="button" disabled={!available}>{t('demos.play')}</button>
        </section>;
      })}
    </div>
  </section>;
}
