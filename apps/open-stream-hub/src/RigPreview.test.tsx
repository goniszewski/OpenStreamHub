import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import { RigPreview } from './RigPreview';
import { DeviceFeed } from './device-feed';
import { baseOrientationStore } from './base-orientation';
import { moduleRotationStore } from './module-rotation';
import { basePinsStore } from './base-pins';
import { LightingStateStore, reportedCyclingColour } from './lighting-state';
import type { DeviceEditor } from './useDeviceEditor';

const stateFor = (bindings: unknown[] = []) => {
  const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Rig', bindings } as never);
  return new ProfileSession(() => new Promise(() => {}), profile, false).getState();
};
const editorFor = (state: ReturnType<typeof stateFor>, module = 'm003k', control = 0) => ({
  selection: { module, control, gesture: 'clockwise', serial: '' }, faderPosition: 50, keypadPath: [keypadRoot], currentKeypadPage: state.draft.profile.keypad!.pages[0],
} as unknown as DeviceEditor);
const listen = async () => () => {};
const feedOf = async (devices: unknown[]) => {
  const feed = new DeviceFeed((async (command: string) => command === 'list_live_devices' ? devices : 'live') as never, listen, true, callback => callback());
  feed.setAutoDetect(true); feed.connect();
  await new Promise(resolve => setTimeout(resolve, 0));
  return feed;
};
const positioned = [
  { path_hex: 'keys', module: 'm4315', serial_number: null, base_cell: { col: 0, row: -1 } },
  { path_hex: 'rollers', module: 'm032le', serial_number: null, base_cell: { col: 0, row: 3 } },
  { path_hex: 'dial', module: 'm131e', serial_number: null, base_cell: { col: 4, row: 0 } },
  { path_hex: 'knobs', module: 'm003k', serial_number: null, base_cell: { col: 4, row: 2 } },
];

describe('RigPreview canvas', () => {
  it('draws nothing without a feed (the browser preview has no modules to lay out)', () => {
    const state = stateFor();
    expect(renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} />)).toBe('');
  });

  it('draws every connected module on the base with its own named controls and the draft bindings', async () => {
    const state = stateFor([
      { module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 5 } },
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
    ]);
    const markup = renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} feed={await feedOf(positioned)} />);
    expect(markup).toContain('Connected base layout');
    expect(markup).toContain('MasterHUB base layout: 15-key IPS display at Top-left (1 row past the top edge), 2-roller scroll module at Bottom-left, Encoder IPS display at Top-right, 3-knob module at Bottom-right');
    // The dial screen shows the profile's dial display face, and each module carries its own hit areas.
    expect(markup).toContain('<canvas width="240" height="240" class="dial-face"');
    expect(markup).toContain('aria-label="Select Knob 1 on the base" aria-pressed="true"');
    expect(markup).toContain('aria-label="Select Knob 2 on the base" aria-pressed="false"');
    expect(markup).toContain('aria-label="Select Key 15 on the base"');
    expect(markup).toContain('aria-label="Select Roller 1 on the base"');
    expect(markup).toMatch(/image-control round bound selected"[^>]*aria-label="Select Knob 1 on the base"/);
    // A module is also pickable by its caption, and the old rig list and its tools are gone.
    expect(markup).toContain('class="base-layout-name" data-picked="true"');
    expect(markup).not.toContain('Add module');
    expect(markup).not.toContain('in rig');
    expect(markup).not.toContain('Move 15-key IPS display left');
  });

  it('renders live state on the module images and keeps live input read-only', async () => {
    const state = stateFor();
    const handlers: Record<string, (event: { payload: unknown }) => void> = {};
    const feed = new DeviceFeed((async (command: string) => command === 'list_live_devices' ? positioned : 'live') as never, async (event, handler) => { handlers[event] = handler; return () => {}; }, true, callback => callback());
    feed.connect(); await new Promise(resolve => setTimeout(resolve, 0));
    const send = (module: string, control: number, gesture: string) => handlers['device://input']({ payload: { feed_id: 'f', module, serial_number: null, event: { timestamp_micros: 1, control, gesture } } });
    handlers['device://status']({ payload: { feed_id: 'f', state: 'running', source: 'demo', module: null } });
    send('m4315', 4, 'pressed'); send('m003k', 0, 'clockwise'); send('m003k', 0, 'clockwise'); send('m131e', 0, 'right');
    const markup = renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} feed={feed} />);
    expect(markup).toMatch(/image-control key-display pressed/);
    expect(markup).toContain('rotate(30deg)');
    // The dial's needle is fitted inside its glass; the knobs' keeps the full length.
    expect(markup).toContain('live-pointer screen');
    expect(markup).toContain('<span class="live-pointer" style="transform:rotate(30deg)"');
    expect(markup).toContain('live-badge');
    expect(markup).toContain('Stop');
  });

  it('names the USB port chain of modules whose position is not detected', async () => {
    const state = stateFor();
    const feed = await feedOf([{ path_hex: 'ab12', module: 'm005s', serial_number: 'S9', port_chain: '1.1.9.9' }]);
    const markup = renderToStaticMarkup(<RigPreview editor={editorFor(state, 'm005s')} state={state} feed={feed} />);
    expect(markup).toContain('USB port chain 1.1.9.9');
  });

  it('shows modules without a detected position in a row beside the base, with the same controls', async () => {
    const state = stateFor();
    const markup = renderToStaticMarkup(<RigPreview editor={editorFor(state, 'm005s')} state={state}
      feed={await feedOf([{ path_hex: 'ab12', module: 'm005s', serial_number: 'S9' }])} />);
    expect(markup).toContain('Position not detected');
    expect(markup).not.toContain('USB port chain');
    expect(markup).toContain('base-layout-module loose selected');
    expect(markup).toContain('5x Fader Precision on the base');
    // Faders are vertical sliders with a value, not toggle buttons.
    const faders = markup.match(/<div[^>]*role="slider"[^>]*>/g) ?? [];
    expect(faders).toHaveLength(5);
    for (const fader of faders) {
      expect(fader).toContain('aria-orientation="vertical"');
      expect(fader).toContain('aria-valuemin="0"');
      expect(fader).toContain('aria-valuemax="100"');
      expect(fader).toContain('aria-valuenow="50"');
      expect(fader).toContain('tabindex="0"');
      expect(fader).not.toContain('aria-pressed');
    }
  });

  it('edits the picked unit of a kind: two knob modules are told apart by the shared pick', async () => {
    const state = stateFor();
    const feed = await feedOf([
      { path_hex: 'k1', module: 'm003k', serial_number: 'MHKB03AA221252301179', base_cell: { col: 0, row: 0 } },
      { path_hex: 'k2', module: 'm003k', serial_number: 'MHKB03AA221252309999', base_cell: { col: 2, row: 0 } },
    ]);
    const markupFor = (pickedPath: string | null) => renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} feed={feed} pickedPath={pickedPath} />);
    const selectedCells = (markup: string) => (markup.match(/base-layout-module[^"]* selected/g) ?? []).length;
    const second = markupFor('k2');
    expect(selectedCells(second)).toBe(1);
    // Only the picked unit outlines the selected control.
    expect(second.match(/image-control[^"]* selected/g)).toHaveLength(1);
    expect(second).toMatch(/class="base-layout-name" data-picked="false"[^>]*><span>3-knob module 1\/2<\/span><span[^>]*aria-label="Serial number MHKB03AA221252301179"/);
    expect(second).toMatch(/class="base-layout-name" data-picked="true"[^>]*><span>3-knob module 2\/2<\/span><span[^>]*aria-label="Serial number MHKB03AA221252309999"/);
    expect(second).not.toContain('…1179');
    // A pick that is not a unit of the editor's kind falls back to the first unit of that kind.
    expect(markupFor('gone')).toMatch(/class="base-layout-name" data-picked="true"[^>]*><span>3-knob module 1\/2<\/span><span[^>]*aria-label="Serial number MHKB03AA221252301179"/);
    expect(markupFor(null)).toMatch(/class="base-layout-name" data-picked="true"[^>]*><span>3-knob module 1\/2<\/span><span[^>]*aria-label="Serial number MHKB03AA221252301179"/);
  });

  it('stands the whole layout up when the base is vertical, turning each module with it', async () => {
    const state = stateFor();
    baseOrientationStore.set(90);
    try {
      const markup = renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} feed={await feedOf(positioned)} />);
      expect(markup).toContain('USB-C ports at the top and right');
      expect(markup).toContain('--cols:5');
      expect(markup).toContain('--rows:6');
      expect(markup).toContain('rig-stage-turn');
      expect(markup).toMatch(/aria-pressed="true"[^>]*>90°</);
      expect(markup).toContain('rig-stage-turn r90');
    } finally { baseOrientationStore.set(0); }
  });

  it('lets one module keep its own rotation while the others follow the base, with a mark and a reset', async () => {
    const state = stateFor();
    const render = async () => renderToStaticMarkup(<RigPreview editor={editorFor(state, 'm032le')} state={state} feed={await feedOf(positioned)} pickedPath="rollers" />);
    let markup = await render();
    expect(markup).toContain('Rotation of');
    expect(markup).toContain('Follow base (0°)');
    expect(markup).not.toContain('Overridden');
    expect(markup).not.toContain('rig-stage-turn');
    moduleRotationStore.set('path:rollers', 90);
    try {
      markup = await render();
      expect(markup).toContain('Overridden');
      // Only the rollers turn; the base and the other modules stay at 0°.
      expect(markup.match(/rig-stage-turn r90/g)).toHaveLength(1);
      expect(markup).not.toContain('rig-stage-turn r180');
    } finally { moduleRotationStore.set('path:rollers', null); }
    expect(await render()).not.toContain('Overridden');
  });

  it('draws the base\'s pogo pins under the modules, with controls to turn or mirror them and to hide them', async () => {
    const state = stateFor();
    const render = async () => renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} feed={await feedOf(positioned)} />);
    let markup = await render();
    expect(markup).toContain('data-testid="pogo-pins"');
    expect(markup).toContain('aria-label="Pogo pins"');
    expect(markup).toContain('Turn 180°');
    expect(markup).toContain('Mirror');
    expect(markup.match(/class="pogo-pad"/g)).toHaveLength(27);
    basePinsStore.set({ show: false });
    try {
      markup = await render();
      expect(markup).not.toContain('data-testid="pogo-pins"');
      expect(markup).not.toContain('Turn 180°');
    } finally { basePinsStore.set({ show: true }); }
  });

  it('keeps only the live controls when the layout is off (the standalone module view is showing)', async () => {
    const state = stateFor();
    const markup = renderToStaticMarkup(<RigPreview editor={editorFor(state)} state={state} feed={await feedOf(positioned)} layout={false} />);
    expect(markup).toContain('Auto-detect modules');
    expect(markup).not.toContain('Connected base layout');
    expect(markup).not.toContain('image-control');
  });

  it('lights a knob ring and a fader glow strip from the stored lighting', async () => {
    const state = stateFor();
    const editor = editorFor(state);
    const make = (module: string) => feedOf([{ path_hex: 'ab12', module, serial_number: null }]);
    const lighting = new LightingStateStore(null);
    lighting.record(['ab12'], () => ({ kind: 'led', leds: [{ colour: '#ff0000', effect: 1, brightness: 100 }, null, null] }));
    const knobsMarkup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={await make('m003k')} lighting={lighting} />);
    expect(knobsMarkup).toContain('module-art-inline');
    expect(knobsMarkup).toContain('color="#ff0000"');
    const faders = new LightingStateStore(null);
    faders.record(['ab12'], () => ({ kind: 'led', leds: [null, { colour: '#00ff00', effect: 1, brightness: 90 }, null, null, null] }));
    const fadersMarkup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={await make('m005s')} lighting={faders} />);
    expect(fadersMarkup).toContain('fill="#00ff00" opacity="1"');
    // An off LED keeps the art's resting look.
    expect(fadersMarkup).not.toContain('fill="#ff0000" opacity="1"');
    // A colour-shifting effect is drawn from one saturated colour, whatever colour was stored (white would stay white).
    const rainbow = new LightingStateStore(null);
    rainbow.record(['ab12'], () => ({ kind: 'led', leds: [{ colour: '#ffffff', effect: 9, brightness: 100 }, null, null] }));
    const rainbowMarkup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={await make('m003k')} lighting={rainbow} />);
    expect(rainbowMarkup).toContain(`color="${reportedCyclingColour}"`);
    expect(rainbowMarkup).not.toContain('color="#ffffff"');
  });
});
