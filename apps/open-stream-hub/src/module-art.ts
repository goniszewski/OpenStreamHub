import fadersSource from './art/faders.svg?raw';
import knobSource from './art/knob.svg?raw';

// The module artwork is SVG. The 15-key and encoder art is drawn in two layers: the module itself
// (`/modules/keys.svg`, `/modules/encoder.svg`) and a transparent glass layer (`*-glass.svg`). A stage puts the
// display content between the two, so pictures sit "under the glass". The 5-fader art is rendered from source
// here because its caps move with the fader values, and the 3-knob art because a ring takes its LED colour.

// Art units of faders.svg (1774 x 887): a cap centre runs from `faderCapBottom` (value 0) up to `faderCapTop`
// (maximum), inside a slot that is `faderSlot` tall.
const faderCapBottom = 675;
const faderCapTop = 305;
const faderSlot = { top: 205, bottom: 775 };
const faderRestingCap = 490;

export const faderArtAspect = 2;
// How far in from the slot's ends the cap centre stops, as a fraction of the slot. A hit area that spans the
// slot maps the pointer onto the cap centre's range with this inset at both ends.
export const faderTrackInset = (faderCapTop - faderSlot.top) / (faderSlot.bottom - faderSlot.top);

export const defaultFaderValues: readonly number[] = [50, 50, 50, 50, 50];

export function faderCapCentre(value: number, maximum = 100): number {
  const ratio = Math.min(Math.max(value / maximum, 0), 1);
  return faderCapBottom - ratio * (faderCapBottom - faderCapTop);
}

// The fader art with each cap at its value and each glow strip lit in its LED's colour
// (a null entry stays off, 'unknown' shows a pale grey placeholder). `idPrefix` keeps gradient ids unique when several are inline on one page.
export function fadersSvg(values: readonly number[], idPrefix = '', maximum = 100, glows?: readonly (string | null)[]): string {
  let index = 0;
  const moved = fadersSource.replace(new RegExp(`class="cap" transform="translate\\(0 ${faderRestingCap}\\)`, 'g'), () => {
    const value = values[index++] ?? defaultFaderValues[0];
    return `class="cap" transform="translate(0 ${Math.round(faderCapCentre(value, maximum) * 10) / 10})`;
  });
  // The strip lights only the part of the track below the cap, so it shrinks as the fader goes down.
  let glowIndex = 0;
  const lit = glows ? moved.replace(/<rect class="glow" x="-9" y="211" width="18" height="558" rx="9" fill="#8a3cff" opacity="0"/g, match => {
    const at = glowIndex++;
    const colour = glows[at];
    if (!colour) return match;
    // The cap's lower edge is about 98 art units under its centre.
    const top = Math.round(faderCapCentre(values[at] ?? defaultFaderValues[0], maximum) + 98);
    const height = Math.max(0, 769 - top);
    const paint = colour === 'unknown' ? 'fill="#a4a4b6" opacity=".5"' : `fill="${colour}" opacity="1"`;
    return `<rect class="glow" x="-9" y="${top}" width="18" height="${height}" rx="9" ${paint}`;
  }) : moved;
  if (!idPrefix) return lit;
  return lit.replace(/\bid="([^"]+)"/g, `id="${idPrefix}$1"`).replace(/url\(#([^)]+)\)/g, `url(#${idPrefix}$1)`);
}

// The knob art with each ring lit in its LED's colour (a null keeps the resting colour).
// The rings take `currentColor`, so one `color` per knob group is the whole change.
export function knobSvg(colours: readonly (string | null)[], idPrefix = ''): string {
  const lit = knobSource.replace(/<g id="knob-(\d)"/g, (match, index: string) => {
    const colour = colours[Number(index)];
    if (colour === 'unknown') return `${match} color="#6b6b78"`;
    return colour ? `${match} color="${colour}"` : match;
  });
  if (!idPrefix) return lit;
  return lit.replace(/\bid="([^"]+)"/g, `id="${idPrefix}$1"`).replace(/url\(#([^)]+)\)/g, `url(#${idPrefix}$1)`);
}

const dataUrls = new Map<string, string>();
// A standalone image of the fader art, for places that show a module as a plain picture.
export function fadersDataUrl(values: readonly number[]): string {
  const key = values.join(',');
  let url = dataUrls.get(key);
  if (!url) {
    url = `data:image/svg+xml,${encodeURIComponent(fadersSvg(values))}`;
    dataUrls.set(key, url);
    if (dataUrls.size > 64) dataUrls.delete(dataUrls.keys().next().value!);
  }
  return url;
}

// The glass layer that goes over a display module's display content, or undefined for modules without displays.
export function moduleGlass(id: string): string | undefined {
  return id === 'm4315' ? '/modules/keys-glass.svg' : id === 'm131e' ? '/modules/encoder-glass.svg' : undefined;
}

// A standalone image of the knob art, for places that show a module as a plain picture. The source lives
// beside the fader art because the lit rings are drawn from it; the plain picture is the same document.
export function knobDataUrl(): string {
  const key = 'knob';
  let url = dataUrls.get(key);
  if (!url) {
    url = `data:image/svg+xml,${encodeURIComponent(knobSvg([]))}`;
    dataUrls.set(key, url);
  }
  return url;
}
