import { afterEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { DemosPage } from '../DemosPage';
import { i18n } from '../i18n-react';
import type { DemoRuntime } from './demo-runtime';

afterEach(() => i18n.setLanguage('en'));
const render = (desktop: boolean, modules: string[]) => renderToStaticMarkup(<DemosPage desktop={desktop} devices={modules.map(module => ({ module }))} />);

describe('DemosPage', () => {
  it('lists tic-tac-toe, says what it needs and that profile actions pause, and disables Play with no module', () => {
    const markup = render(true, ['m131e']);
    expect(markup).toContain('Tic-tac-toe');
    expect(markup).toContain('Needs the 15-key module: none is connected.');
    expect(markup).toContain('Your profile’s actions are paused while a demo runs.');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Play<\/button>/);
  });
  it('enables Play once a 15-key module is connected in the desktop app', () => {
    const runtime = {} as DemoRuntime;
    const markup = renderToStaticMarkup(<DemosPage desktop devices={[{ module: 'm4315', path_hex: 'ab' }]} runtime={runtime} />);
    expect(markup).toContain('Needs the 15-key module');
    expect(markup).not.toContain('none is connected');
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
