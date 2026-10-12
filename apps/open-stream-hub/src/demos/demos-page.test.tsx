import { afterEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { DemosPage } from '../DemosPage';
import { i18n } from '../i18n-react';
import type { DemoRuntime } from './demo-runtime';

afterEach(() => i18n.setLanguage('en'));
const render = (desktop: boolean, modules: string[]) => renderToStaticMarkup(<DemosPage desktop={desktop} devices={modules.map(module => ({ module }))} />);

import { DemoKeys } from './DemoKeys';

describe('DemoKeys', () => {
  it('shows the 15-key module with each key\'s picture and a name for every key', () => {
    const markup = renderToStaticMarkup(<DemoKeys images={Array.from({ length: 15 }, (_, key) => key === 2 ? 'QUJD' : null)} label={key => `Key ${key + 1}`} onPress={() => {}} />);
    expect(markup).toContain('15x Key IPS Display');
    expect(markup.match(/image-control key-display demo-key/g)).toHaveLength(15);
    expect(markup).toContain('aria-label="Key 15"');
    expect(markup).toContain('src="data:image/png;base64,QUJD"');
    expect(markup.match(/<img src="data:image/g)).toHaveLength(1);
  });
});

describe('DemosPage', () => {
  it('lists tic-tac-toe, says what it needs and that profile actions pause, and disables Play with no module', () => {
    const markup = render(true, ['m131e']);
    expect(markup).toContain('Tic-tac-toe');
    expect(markup).toContain('Needs 15-key module: not connected: 15-key module.');
    expect(markup).toContain('class="demo-tag">15-key module');
    expect(markup).toContain('Your profile’s actions are paused while a demo runs.');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Play<\/button>/);
  });
  it('enables Play once a 15-key module is connected in the desktop app', () => {
    const runtime = {} as DemoRuntime;
    const markup = renderToStaticMarkup(<DemosPage desktop devices={[{ module: 'm4315', path_hex: 'ab' }]} runtime={runtime} />);
    expect(markup).toContain('Needs 15-key module.');
    expect(markup).toContain('class="demo-tag present">15-key module');
    expect(markup).not.toContain('not connected: ');
    expect(markup).not.toMatch(/<button[^>]*disabled/);
  });
  it('is read-only in the browser preview even with a module listed', () => {
    const markup = render(false, ['m4315']);
    expect(markup).toContain('cannot play them');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Play<\/button>/);
  });
  it('is translated', () => {
    i18n.setLanguage('pl');
    const markup = render(true, []);
    expect(markup).toContain('Kółko i krzyżyk');
    expect(markup).toContain('Akcje profilu są wstrzymane');
  });
});
