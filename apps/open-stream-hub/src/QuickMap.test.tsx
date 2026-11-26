import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { QuickMapButton, QuickMapDrawing, layoutLegend, paperPalette, placeModules, quickMapFileName, quickMapSvg } from './QuickMap';
import { quickMap } from './quick-map-model';
import { starters } from './starter-model';
import { browserProfile } from './ui-model';

describe('quick map drawing', () => {
  it('is a labelled image with a title, a description and a callout per bound control', () => {
    const map = quickMap(browserProfile);
    const svg = renderToStaticMarkup(<QuickMapDrawing map={map} />);
    expect(svg).toContain('role="img"');
    expect(svg).toContain('Quick map of the profile Everyday');
    expect(svg).toContain('Knob 1');
    expect(svg).toContain('Clockwise: Output volume +5 points');
  });
  it('the saved copy has fixed paper colours rather than theme variables', () => {
    const svg = quickMapSvg(quickMap(browserProfile));
    expect(svg).toContain(paperPalette.paper);
    expect(svg).not.toContain('var(--');
  });
  it('the on-screen copy follows the theme', () => {
    expect(renderToStaticMarkup(<QuickMapDrawing map={quickMap(browserProfile)} />)).toContain('var(--text)');
  });
  it('draws every starter profile with all its legend lines', () => {
    for (const template of starters) {
      const map = quickMap(template.profile);
      const svg = renderToStaticMarkup(<QuickMapDrawing map={map} />);
      for (const line of map.legend) expect(svg).toContain(line.name.replace(/&/g, '&amp;'));
    }
  });
  it('says so when nothing is bound', () => {
    expect(renderToStaticMarkup(<QuickMapDrawing map={quickMap({ schema_version: 1, name: 'x', bindings: [] })} />)).toContain('nothing bound yet');
  });
  it('wraps modules onto a second row and splits the legend in two columns', () => {
    const map = quickMap(starters.find(item => item.id === 'mac-basics')!.profile);
    const { placed } = placeModules(map.modules);
    expect(placed.every(item => item.x >= 20 && item.x < 780)).toBe(true);
    expect(new Set(placed.map(item => item.y)).size).toBeGreaterThan(1);
    const layout = layoutLegend(map.legend);
    expect(layout.columns[0].length + layout.columns[1].length).toBe(map.legend.length);
  });
  it('names the saved file after the profile', () => {
    expect(quickMapFileName('Terminal and AI agents')).toBe('terminal-and-ai-agents-quick-map.svg');
    expect(quickMapFileName('///')).toBe('profile-quick-map.svg');
  });
  it('the header button is a labelled button opening a dialog', () => {
    const html = renderToStaticMarkup(<QuickMapButton profile={browserProfile} />);
    expect(html).toContain('aria-haspopup="dialog"');
    expect(html).toContain('>Quick map<');
  });
});
