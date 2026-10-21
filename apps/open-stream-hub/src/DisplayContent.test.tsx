import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { KeyDisplayContent, DialFace } from './DisplayContent';
import type { Profile } from './contracts';
import { dialFacePixels, wrapDialName } from './dial-face';

const withDialDisplay = (profile: Profile, content: string, image?: string): Profile =>
  ({ ...profile, dial_display: { content, ...(image ? { image } : {}) } } as Profile);
const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
// One pixel of the 240×240 face, as [r, g, b].
const pixelAt = (pixels: Uint8ClampedArray, x: number, y: number): [number, number, number] => {
  const at = (y * 240 + x) * 4;
  return [pixels[at], pixels[at + 1], pixels[at + 2]];
};

// Verify the previews reflect saved content, rather than placeholder artwork.
describe('display content', () => {
  it('renders the current action label and its mute icon', () => {
    const markup = renderToStaticMarkup(<KeyDisplayContent slot={{type:'action', label:'Quiet mode', bindings:[{gesture:'pressed', action:{type:'toggle_mute'}}]}} pages={[]} depth={0} back={false} />);
    expect(markup).toContain('Quiet mode');
    expect(markup).toContain('lucide-volume-x');
  });
  it('uses the destination folder name and generated Back depth', () => {
    expect(renderToStaticMarkup(<KeyDisplayContent slot={{type:'folder', page_id:'tools'}} pages={[{id:'tools', name:'Tools', slots:[]}]} depth={1} back={false} />)).toContain('Tools');
    const back = renderToStaticMarkup(<KeyDisplayContent slot={null} pages={[]} depth={3} back />);
    expect(back).toContain('L3');
    expect(back).toContain('lucide-arrow-left');
  });
});

describe('dial face', () => {
  it('draws a canvas face by default and for the explicit choices', () => {
    // The default for a profile with nothing on the dial is the analog clock.
    expect(renderToStaticMarkup(<DialFace profile={flat} />)).toContain('<canvas width="240" height="240"');
    for (const content of ['function', 'clock', 'analog_clock', 'cpu'] as const) {
      const markup = renderToStaticMarkup(<DialFace profile={withDialDisplay(flat, content)} />);
      expect(markup).toContain('<canvas width="240" height="240"');
    }
  });
  it('shows the uploaded picture itself and nothing when off', () => {
    const image = renderToStaticMarkup(<DialFace profile={withDialDisplay(flat, 'image', 'AAAA')} />);
    expect(image).toContain('<img class="dial-face" src="data:image/jpeg;base64,AAAA"');
    expect(renderToStaticMarkup(<DialFace profile={withDialDisplay(flat, 'off')} />)).not.toContain('<canvas');
    // Image mode without a picture yet: nothing drawn, no broken image.
    expect(renderToStaticMarkup(<DialFace profile={withDialDisplay(flat, 'image')} />)).not.toContain('<img');
  });
  it('paints the faces with the device palette', () => {
    // Every face starts black, and the analog clock puts its orange hand
    // through the centre.
    const analog = dialFacePixels({ kind: 'analog_clock', time: { hour: 9, minute: 41 } });
    expect(pixelAt(analog, 0, 0)).toEqual([0, 0, 0]);
    expect(pixelAt(analog, 120, 120)).toEqual([230, 120, 40]);
    // The digital clock lights the accent colon between the digit pairs.
    const clock = dialFacePixels({ kind: 'clock', time: { hour: 9, minute: 41 } });
    expect(pixelAt(clock, 119, 106)).toEqual([40, 140, 220]);
    // The function face ring and the first-function dot are accent blue.
    const face = dialFacePixels({ kind: 'function', name: 'Volume', index: 0, count: 2 });
    expect(pixelAt(face, 120, 4)).toEqual([40, 140, 220]);
    // The CPU ring reads accent blue below the high mark.
    const cpu = dialFacePixels({ kind: 'cpu', percent: 37 });
    expect(pixelAt(cpu, 120, 8)).toEqual([40, 140, 220]);
  });
  it('wraps long names like the device faces', () => {
    expect(wrapDialName('Volume')).toEqual(['Volume']);
    expect(wrapDialName('Streaming and video')).toEqual(['Streaming', 'and video']);
    expect(wrapDialName('Analog clock face')).toEqual(['Analog', 'clock face']);
    // Over the 22-character budget: cut with a trailing dot before splitting.
    expect(wrapDialName('A very long function name here')).toEqual(['A very long', 'function .']);
  });
});

describe('keyLabelLines', () => {
  it('wraps a label as the module does: large for two short lines, else small, cut with a dot', async () => {
    const { keyLabelLines } = await import('./DisplayContent');
    expect(keyLabelLines('Record')).toEqual({ lines: ['Record'], scale: 2 });
    expect(keyLabelLines('Mute mic')).toEqual({ lines: ['Mute', 'mic'], scale: 2 });
    expect(keyLabelLines('Toggle microphone')).toEqual({ lines: ['Toggle', 'microphone'], scale: 1 });
    expect(keyLabelLines('a'.repeat(40)).lines.at(-1)).toMatch(/\.$/);
    expect(keyLabelLines('').lines).toEqual([]);
  });
});
