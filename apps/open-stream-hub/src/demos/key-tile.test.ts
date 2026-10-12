import { describe, expect, it } from 'vitest';
import { drawTile, tilePng, tileSize } from './key-tile';
import type { Drawing } from './key-tile';

function recorder() {
  const calls: string[] = [];
  const ctx = new Proxy({ fillStyle: '', strokeStyle: '', lineWidth: 0, lineCap: '', font: '', textAlign: '', textBaseline: '' } as Record<string, unknown>, {
    get: (target, name: string) => name in target ? target[name] : (...args: unknown[]) => { calls.push(`${name}(${args.join(',')})`); },
    set: (target, name: string, value) => { target[name] = value; return true; },
  }) as unknown as Drawing;
  return { ctx, calls };
}

describe('key tiles', () => {
  it('leaves a blank key black and draws marks, labels and scores', () => {
    const blank = recorder();
    drawTile(blank.ctx, { tone: 'blank' });
    expect(blank.calls).toEqual([`fillRect(0,0,${tileSize},${tileSize})`]);
    const x = recorder();
    drawTile(x.ctx, { tone: 'x', mark: 'X' });
    expect(x.calls.filter(call => call.startsWith('lineTo'))).toHaveLength(2);
    const o = recorder();
    drawTile(o.ctx, { tone: 'win', mark: 'O' });
    expect(o.calls.some(call => call.startsWith('arc('))).toBe(true);
    const label = recorder();
    drawTile(label.ctx, { tone: 'button', text: 'New', sub: 'game' });
    expect(label.calls.filter(call => call.startsWith('fillText'))).toHaveLength(2);
    const score = recorder();
    drawTile(score.ctx, { tone: 'score', mark: 'X', text: '3' });
    expect(score.calls.filter(call => call.startsWith('fillText')).map(call => call.split(',')[0])).toEqual(['fillText(X', 'fillText(3']);
  });

  it('draws a mole with eyes and nose, and a tick or a cross for the verdict', () => {
    const mole = recorder();
    drawTile(mole.ctx, { tone: 'mole' });
    expect(mole.calls.filter(call => call.startsWith('fill()'))).toHaveLength(4);
    const tick = recorder();
    drawTile(tick.ctx, { tone: 'hit' });
    expect(tick.calls.filter(call => call.startsWith('lineTo'))).toHaveLength(2);
    const cross = recorder();
    drawTile(cross.ctx, { tone: 'miss' });
    expect(cross.calls.filter(call => call.startsWith('lineTo'))).toHaveLength(2);
  });

  it('gives the page canvas picture as base64 and null where there is no canvas', () => {
    const { ctx } = recorder();
    const canvas = { getContext: () => ctx, toDataURL: () => 'data:image/png;base64,QUJD', width: 0, height: 0 } as unknown as HTMLCanvasElement;
    expect(tilePng({ tone: 'empty' }, () => canvas)).toBe('QUJD');
    expect(canvas.width).toBe(tileSize);
    expect(tilePng({ tone: 'empty' }, () => null)).toBeNull();
    expect(tilePng({ tone: 'empty' }, () => ({ getContext: () => null }) as unknown as HTMLCanvasElement)).toBeNull();
  });
});
