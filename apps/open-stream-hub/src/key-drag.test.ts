import { describe, expect, it } from 'vitest';
import { keyDragMime, offersKeyDrag, readKeyDrag, writeKeyDrag } from './key-drag';

describe('key drag payload', () => {
  it('round-trips the source key', () => {
    const data = new Map<string, string>();
    const transfer = { setData: (name: string, value: string) => { data.set(name, value); }, effectAllowed: '' };
    writeKeyDrag(transfer as unknown as DataTransfer, { path: ['root', 'fx'], control: 4 });
    expect(data.get(keyDragMime)).toBe(JSON.stringify({ path: ['root', 'fx'], control: 4 }));
    expect(transfer.effectAllowed).toBe('copyMove');
    expect(offersKeyDrag({ types: [keyDragMime] })).toBe(true);
    expect(offersKeyDrag({ types: ['text/plain'] })).toBe(false);
    expect(readKeyDrag({ getData: name => data.get(name) ?? '' })).toEqual({ path: ['root', 'fx'], control: 4 });
  });

  it('refuses foreign or malformed payloads', () => {
    const reading = (value: string) => readKeyDrag({ getData: () => value });
    expect(reading('')).toBeNull();
    expect(reading('not json')).toBeNull();
    expect(reading('null')).toBeNull();
    expect(reading('{"path":"root","control":1}')).toBeNull();
    expect(reading('{"path":[2],"control":1}')).toBeNull();
    expect(reading('{"path":[],"control":1}')).toBeNull();
    expect(reading('{"path":["root"],"control":15}')).toBeNull();
    expect(reading('{"path":["root"],"control":1.5}')).toBeNull();
    expect(readKeyDrag({ getData: () => JSON.stringify({ path: ['root'], control: 14 }) })).toEqual({ path: ['root'], control: 14 });
  });
});
