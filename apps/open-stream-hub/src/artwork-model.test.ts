import { describe, expect, it, vi } from 'vitest';
import {
  artworkName, preparedQuality, buildArtworkRequest, buildSaveRequest, bytesToBase64, clampQuarterTurns, createLatestRunner, defaultArtworkOptions, formatHexColour,
  maxSourceBytes, parseHexColour, prepareArtwork, readSourceFile, saveArtwork,
} from './artwork-model';
import type { ArtworkOptions, PrepareRequest } from './artwork-model';
import contracts from '../fixtures/artwork-prepare-request.json';
import prepareResult from '../fixtures/artwork-prepare-result.json';
import saveRequest from '../fixtures/artwork-save-request.json';
import saveResult from '../fixtures/artwork-save-result.json';

const options = (patch: Partial<ArtworkOptions> = {}): ArtworkOptions => ({ ...defaultArtworkOptions, ...patch });

describe('colour helpers', () => {
  it('parses 3 and 6 digit hex with or without #', () => {
    expect(parseHexColour('#000000')).toEqual([0, 0, 0]);
    expect(parseHexColour('ff8000')).toEqual([255, 128, 0]);
    expect(parseHexColour('#FA0')).toEqual([255, 170, 0]);
    expect(parseHexColour(' abc ')).toEqual([170, 187, 204]);
  });
  it('rejects invalid input', () => {
    for (const text of ['', '#', '#12', '#1234', '#12345', '#1234567', '#ggg', 'red', '#12 456', '0x112233']) expect(parseHexColour(text), text).toBeNull();
  });
  it('formats and round-trips', () => {
    expect(formatHexColour([0, 128, 255])).toBe('#0080ff');
    expect(formatHexColour([300, -4, 1.4])).toBe('#ff0001');
    expect(parseHexColour(formatHexColour([1, 2, 3]))).toEqual([1, 2, 3]);
  });
});

describe('clamping', () => {
  it('wraps quarter turns into 0..3', () => {
    expect([clampQuarterTurns(0), clampQuarterTurns(4), clampQuarterTurns(-1), clampQuarterTurns(2.4), clampQuarterTurns(NaN)]).toEqual([0, 0, 3, 2, 0]);
  });
});

describe('buildArtworkRequest', () => {
  it('needs an image', () => {
    expect(buildArtworkRequest(null, options())).toMatchObject({ ok: false });
    expect(buildArtworkRequest('', options())).toMatchObject({ ok: false });
  });
  it('rejects a bad colour', () => {
    const outcome = buildArtworkRequest('AAAA', options({ background: 'blue' }));
    expect(outcome).toEqual({ ok: false, error: expect.stringContaining('hex') });
  });
  it('sends every field with the documented defaults', () => {
    expect(buildArtworkRequest('AAAA', options())).toEqual({
      ok: true, request: { image_base64: 'AAAA', target: 'dial', quality: 100, fit: 'contain', quarter_turns: 0, background: [0, 0, 0] },
    });
  });
  it.each([
    [{ target: 'keypad' as const }, { target: 'keypad' }],
    [{ fit: 'cover' as const }, { fit: 'cover' }],
    [{ fit: 'stretch' as const }, { fit: 'stretch' }],
    [{ quarterTurns: 3 }, { quarter_turns: 3 }],
    [{ quarterTurns: 5 }, { quarter_turns: 1 }],
    [{ background: '#0f0' }, { background: [0, 255, 0] }],
  ])('maps %j', (patch, expected) => {
    const outcome = buildArtworkRequest('AAAA', options(patch));
    expect(outcome.ok && outcome.request).toMatchObject(expected);
  });
  it('adds a trimmed name for saving', () => {
    const outcome = buildArtworkRequest('AAAA', options());
    if (!outcome.ok) throw new Error('unreachable');
    expect(buildSaveRequest(outcome.request, '  Cover  ').name).toBe('Cover');
    expect(artworkName('my.cover.png')).toBe('my.cover');
    expect(artworkName('.png')).toBe('artwork');
    expect(artworkName('plain')).toBe('plain');
  });
});

describe('base64', () => {
  it('encodes small inputs like btoa', () => {
    expect(bytesToBase64(new Uint8Array([104, 101, 108, 108, 111]))).toBe('aGVsbG8=');
    expect(bytesToBase64(new Uint8Array())).toBe('');
  });
  it('handles multi-megabyte buffers without overflowing the stack', () => {
    const bytes = new Uint8Array(maxSourceBytes);
    for (let index = 0; index < bytes.length; index += 1) bytes[index] = index % 251;
    const text = bytesToBase64(bytes);
    expect(text.length).toBe(Math.ceil(bytes.length / 3) * 4);
    expect(Array.from(Uint8Array.from(atob(text.slice(0, 4000)), c => c.charCodeAt(0)).slice(0, 5))).toEqual([0, 1, 2, 3, 4]);
    expect(atob(text).length).toBe(bytes.length);
  });
});

describe('readSourceFile', () => {
  const file = (bytes: number[] | number, type = 'image/png') => {
    const data = typeof bytes === 'number' ? new Uint8Array(bytes) : Uint8Array.from(bytes);
    return { size: data.length, type, arrayBuffer: async () => data.buffer as ArrayBuffer };
  };
  it('reads a file', async () => {
    expect(await readSourceFile(file([1, 2, 3]))).toEqual({ ok: true, base64: 'AQID', mime: 'image/png', size: 3 });
  });
  it('rejects empty and oversized files before reading', async () => {
    const big = { size: maxSourceBytes + 1, type: 'image/png', arrayBuffer: vi.fn() };
    expect(await readSourceFile(big)).toEqual({ ok: false, error: expect.stringContaining('8 MiB') });
    expect(big.arrayBuffer).not.toHaveBeenCalled();
    expect(await readSourceFile(file(0))).toMatchObject({ ok: false });
  });
  it('accepts exactly the limit and reports read failures', async () => {
    expect(await readSourceFile(file(maxSourceBytes))).toMatchObject({ ok: true, size: maxSourceBytes });
    expect(await readSourceFile({ size: 4, arrayBuffer: async () => { throw new Error('denied'); } })).toEqual({ ok: false, error: 'Could not read the file: denied' });
  });
});

describe('createLatestRunner', () => {
  const deferred = <T,>() => { let resolve!: (value: T) => void; let reject!: (reason: unknown) => void; const promise = new Promise<T>((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
  it('drops the stale result when a newer run finished first', async () => {
    const runner = createLatestRunner();
    const first = deferred<string>(); const second = deferred<string>();
    const a = runner.run(() => first.promise); const b = runner.run(() => second.promise);
    second.resolve('new'); first.resolve('old');
    expect(await b).toEqual({ ok: true, value: 'new' });
    expect(await a).toBeNull();
  });
  it('drops a stale error and reports a current one', async () => {
    const runner = createLatestRunner();
    const first = deferred<string>();
    const a = runner.run(() => first.promise);
    const b = runner.run(async () => { throw new Error('current failure'); });
    first.reject(new Error('stale failure'));
    expect(await a).toBeNull();
    expect(await b).toEqual({ ok: false, error: 'current failure' });
  });
  it('cancel discards the in-flight result', async () => {
    const runner = createLatestRunner();
    const pending = deferred<number>();
    const run = runner.run(() => pending.promise);
    runner.cancel(); pending.resolve(1);
    expect(await run).toBeNull();
  });
  it('maps string rejections from the backend to messages', async () => {
    expect(await createLatestRunner().run(() => Promise.reject('Animated PNG is not supported'))).toEqual({ ok: false, error: 'Animated PNG is not supported' });
  });
});

describe('commands and shared fixtures', () => {
  it('call the Tauri commands with a single request object', async () => {
    const invoke = vi.fn(async () => prepareResult.prepare_artwork) as never;
    const request = contracts.request as PrepareRequest;
    await prepareArtwork(invoke, request);
    expect(invoke).toHaveBeenCalledWith('prepare_artwork', { request });
    const save = vi.fn(async () => saveResult.save_artwork) as never;
    await saveArtwork(save, saveRequest.request as never);
    expect(save).toHaveBeenCalledWith('save_artwork', { request: saveRequest.request });
  });
  it('requests built here match the Rust request shape', () => {
    const outcome = buildArtworkRequest(contracts.request.image_base64, options({ target: 'keypad', quarterTurns: 1 }));
    expect(outcome).toEqual({ ok: true, request: { ...contracts.request, quality: preparedQuality } });
    expect(Object.keys(saveRequest.request).sort()).toEqual([...Object.keys(contracts.request), 'name'].sort());
    if (outcome.ok) expect(buildSaveRequest(outcome.request, 'Four squares')).toEqual({ ...saveRequest.request, quality: preparedQuality });
  });
  it('results have the documented fields', () => {
    expect(Object.keys(prepareResult.prepare_artwork).sort()).toEqual(['bytes', 'height', 'jpeg_base64', 'width']);
    expect(prepareResult.prepare_artwork).toMatchObject({ width: 72, height: 72 });
    expect(Object.keys(saveResult.save_artwork).sort()).toEqual(['bytes', 'height', 'path', 'width']);
    expect(saveResult.save_artwork.path).toMatch(/\/artwork\/[a-z0-9-]+-(dial|keypad)-\d{8}-\d{6}\.jpg$/);
  });
});
