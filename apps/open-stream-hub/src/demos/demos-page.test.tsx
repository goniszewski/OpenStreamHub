import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { DemosPage } from '../DemosPage';
import type { DemoRuntime } from './demo-runtime';

const render = (desktop: boolean, modules: string[]) => renderToStaticMarkup(<DemosPage desktop={desktop} devices={modules.map(module => ({ module }))} />);

import { DemoKeys } from './DemoKeys';
import { SpeedTestScreen } from './SpeedTestScreen';
import { WhackScreen } from './WhackScreen';

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

describe('SpeedTestScreen', () => {
  it('waits for the Start button before it takes the module, and offers Exit', () => {
    const runtime = { getPhase: () => 'idle' } as unknown as DemoRuntime;
    const markup = renderToStaticMarkup(<SpeedTestScreen runtime={runtime} pathHex="ab" onEnd={() => {}} />);
    expect(markup).toContain('Display speed test');
    expect(markup).toContain('Ready. The test takes about a minute.');
    expect(markup).toContain('>Start the test<');
    expect(markup).toContain('>Exit<');
    expect(markup).not.toContain('<pre');
  });
});

describe('WhackScreen', () => {
  it('offers both modes and waits for Start before anything moves', () => {
    const runtime = { getPhase: () => 'idle' } as unknown as DemoRuntime;
    const markup = renderToStaticMarkup(<WhackScreen runtime={runtime} pathHex="ab" onEnd={() => {}} />);
    expect(markup).toContain('>Reaction test<');
    expect(markup).toContain('A 30-second round');
    expect(markup).toContain('Score 0 · best 0');
    expect(markup).toContain('>Exit<');
    expect(markup.match(/image-control key-display demo-key/g)).toHaveLength(15);
  });
});

describe('DemosPage', () => {
  it('lists tic-tac-toe, says what it needs and that profile actions pause, and disables Play with no module', () => {
    const markup = render(true, ['m131e']);
    expect(markup).toContain('Tic-tac-toe');
    expect(markup).toContain('Display speed test');
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
});
