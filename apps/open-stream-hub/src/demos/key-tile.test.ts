import { describe, expect, it } from 'vitest';
import { drawTile, inkOn, numberFont, tilePng, tileSize } from './key-tile';
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

  it('fills a numbered tile with its colour, its number in the ink that reads on it, smaller for more digits', () => {
    const tile = recorder();
    drawTile(tile.ctx, { tone: 'value', text: '2048', colour: '#ececec' });
    expect(tile.calls).toContain(`fillRect(6,6,${tileSize - 12},${tileSize - 12})`);
    expect(tile.calls.filter(call => call.startsWith('fillText(2048'))).toHaveLength(1);
    expect(tile.ctx.fillStyle).toBe('#181820');
    expect(inkOn('#2e808c')).toBe('#fafafc');
    expect(numberFont('2', 72)).toBe('bold 72px sans-serif');
    expect(numberFont('2048', 72)).toBe('bold 50px sans-serif');
    expect(numberFont('131072', 72)).toBe('bold 33px sans-serif');
    const pointed = recorder();
    drawTile(pointed.ctx, { tone: 'value', text: '8', colour: '#2e808c', arrow: 'left' });
    expect(pointed.calls.slice(-5)).toEqual(['beginPath()', 'moveTo(35,20)', 'lineTo(25,30)', 'lineTo(35,40)', 'stroke()']);
    const empty = recorder();
    drawTile(empty.ctx, { tone: 'empty', arrow: 'down' });
    expect(empty.ctx.strokeStyle).toBe('#6a6a7c');
    const plain = recorder();
    drawTile(plain.ctx, { tone: 'label', text: 'over' });
    expect(plain.calls.filter(call => call.startsWith('fillText'))).toHaveLength(1);
    expect(plain.ctx.strokeStyle).toBe('#000000');
    const score = recorder();
    drawTile(score.ctx, { tone: 'label', label: 'Best', text: '40' });
    expect(score.calls.filter(call => call.startsWith('fillText')).map(call => call.split(',')[0])).toEqual(['fillText(Best', 'fillText(40']);
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
