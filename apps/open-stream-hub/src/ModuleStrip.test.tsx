import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ModuleStrip } from './ModuleStrip';
import { moduleAcceptsDisplayArtwork, moduleArtworkCaption } from './artwork-model';
import type { LightingDevice } from './lighting-model';
import { createModuleSelection } from './module-selection';

const base: LightingDevice = { path_hex: 'bs', module: 'm0064', serial_number: 'MHBS01AA461251601980' };
const dial: LightingDevice = { path_hex: 'dl', module: 'm131e', serial_number: 'MHDD01AA221252300843' };
const faders: LightingDevice = { path_hex: 'fd', module: 'm005s', serial_number: 'MHFD05AA421251601744' };
const keypad: LightingDevice = { path_hex: 'ke', module: 'm4315', serial_number: null };

describe('ModuleStrip', () => {
  it('shows a tile per module with photo, name, serial tail and what lighting it takes', () => {
    const markup = renderToStaticMarkup(<ModuleStrip devices={[base, dial, faders, keypad]} selection={createModuleSelection(null)} />);
    expect(markup).toContain('aria-label="Module"');
    expect(markup).toContain('src="/modules/base.svg"');
    expect(markup).toContain('src="/modules/encoder.png"');
    expect(markup).toContain('src="/modules/faders.png"');
    expect(markup).toContain('<span class="module-tile-serial">…1744</span>');
    expect(markup).toContain('MasterHUB base');
    expect(markup).toContain('Backlight and colour');
    expect(markup).toContain('Colour, effect and brightness · 5 LEDs');
    expect(markup).toContain('Colour, effect and brightness · 1 LED');
    expect(markup).toContain('Not lit yet');
    // With nothing picked, the first dial is; the keypad is dimmed but reachable.
    expect(markup).toMatch(/aria-pressed="true"[^>]*class="module-tile picked"/);
    expect(markup).toContain('aria-label="Encoder IPS display · …0843"');
    expect(markup).toMatch(/class="module-tile unlit"/);
  });
  it('shows the pick from the shared store and flags the marked modules', () => {
    const selection = createModuleSelection(null);
    selection.select('fd');
    selection.mark(['bs', 'fd']);
    const markup = renderToStaticMarkup(<ModuleStrip devices={[base, dial, faders]} selection={selection} />);
    expect(markup).toMatch(/aria-label="5-fader precision module · …1744" class="module-tile picked marked"/);
    expect(markup).toMatch(/aria-label="MasterHUB base · …1980" class="module-tile marked"/);
    expect(markup.match(/Included in Whole kit/g)).toHaveLength(2);
  });
  it('takes another page’s caption and usability rule', () => {
    const markup = renderToStaticMarkup(<ModuleStrip devices={[dial, faders, keypad]} selection={createModuleSelection(null)}
      caption={moduleArtworkCaption} usable={moduleAcceptsDisplayArtwork} />);
    expect(markup).toContain('Round display, 240 px');
    expect(markup).toContain('Key displays, 72 px');
    expect(markup).toContain('No display');
    expect(markup).not.toContain('Not lit yet');
    expect(markup.match(/class="module-tile unlit"/g)).toHaveLength(1);
  });
  it('says so when nothing is connected', () => {
    expect(renderToStaticMarkup(<ModuleStrip devices={[]} selection={createModuleSelection(null)} />)).toContain('No modules are connected.');
  });
});
