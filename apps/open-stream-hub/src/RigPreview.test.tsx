import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import { RigPreview } from './RigPreview';
import { DeviceFeed } from './device-feed';
import { LightingStateStore } from './lighting-state';
import type { DeviceEditor } from './useDeviceEditor';

describe('RigPreview', () => {
  it('draws the draft bindings on the default rig', () => {
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Rig', bindings: [
      { module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 5 } },
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
    ] });
    const session = new ProfileSession(() => new Promise(() => {}), profile, false);
    const state = session.getState();
    const editor = { selection: { module: 'm003k', control: 0, gesture: 'clockwise', serial: '' }, faderPosition: 50, keypadPath: [keypadRoot], currentKeypadPage: state.draft.profile.keypad!.pages[0] } as unknown as DeviceEditor;
    const markup = renderToStaticMarkup(<RigPreview editor={editor} state={state} />);
    // The dial screen shows the profile's dial display face (drawn on a
    // canvas like the device's), not the encoder binding's label.
    expect(markup).toContain('<canvas width="240" height="240" class="dial-face"');
    expect(markup).not.toContain('Volume +5');
    expect(markup).toContain('1 bound');
    expect(markup).toContain('Select Knob 1 in rig');
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain('Move 15-key IPS display left');
  });

  it('renders live state and read-only controls', async () => {
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Rig', bindings: [] });
    const state = new ProfileSession(() => new Promise(() => {}), profile, false).getState();
    const editor = { selection: { module: 'm003k', control: 0, gesture: 'clockwise', serial: '' }, faderPosition: 50, keypadPath: [keypadRoot], currentKeypadPage: state.draft.profile.keypad!.pages[0] } as unknown as DeviceEditor;
    const handlers: Record<string, (event: { payload: unknown }) => void> = {};
    const feed = new DeviceFeed((async () => []) as never, async (event, handler) => { handlers[event] = handler; return () => {}; }, true, callback => callback());
    feed.connect(); await new Promise(resolve => setTimeout(resolve, 0));
    const send = (module: string, control: number, gesture: string) => handlers['device://input']({ payload: { feed_id: 'f', module, serial_number: null, event: { timestamp_micros: 1, control, gesture } } });
    handlers['device://status']({ payload: { feed_id: 'f', state: 'running', source: 'demo', module: null } });
    send('m4315', 4, 'pressed'); send('m003k', 0, 'clockwise'); send('m003k', 0, 'clockwise'); send('m131e', 0, 'right');
    const markup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={feed} />);
    expect(markup).toMatch(/image-control key-display pressed/);
    expect(markup).toContain('rotate(30deg)');
    // The dial's needle is fitted inside its glass; the knobs' keeps the full length.
    expect(markup).toContain('live-pointer screen');
    expect(markup).toContain('<span class="live-pointer" style="transform:rotate(30deg)"');
    expect(markup).toContain('live-badge');
    expect(markup).toContain('Stop');
    expect(markup).toContain('no actions run');
  });

  it('draws connected devices as rig items when auto-detect is on', async () => {
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Rig', bindings: [] });
    const state = new ProfileSession(() => new Promise(() => {}), profile, false).getState();
    const editor = { selection: { module: 'm003k', control: 0, gesture: 'clockwise', serial: '' }, faderPosition: 50, keypadPath: [keypadRoot], currentKeypadPage: state.draft.profile.keypad!.pages[0] } as unknown as DeviceEditor;
    const listen = async () => () => {};
    const make = (autoDetect: boolean) => {
      const feed = new DeviceFeed((async (command: string) => command === 'list_live_devices' ? [{ path_hex: 'ab12', module: 'm005s', serial_number: 'S9' }] : 'live') as never, listen, true, callback => callback());
      feed.setAutoDetect(autoDetect); feed.connect(); return feed;
    };
    const on = make(true); await new Promise(resolve => setTimeout(resolve, 0));
    const markup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={on} />);
    expect(markup).toContain('5x Fader Precision in rig');
    expect(markup).toContain('connected');
    expect(markup).toContain('Auto-detect modules');
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
    const off = make(false); await new Promise(resolve => setTimeout(resolve, 0));
    expect(renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={off} />)).not.toContain('5x Fader Precision in rig');
  });

  it('lights a knob ring and a fader glow strip from the stored lighting', async () => {
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Rig', bindings: [] });
    const state = new ProfileSession(() => new Promise(() => {}), profile, false).getState();
    const editor = { selection: { module: 'm003k', control: 0, gesture: 'clockwise', serial: '' }, faderPosition: 50, keypadPath: [keypadRoot], currentKeypadPage: state.draft.profile.keypad!.pages[0] } as unknown as DeviceEditor;
    const listen = async () => () => {};
    const make = async (module: string) => {
      const feed = new DeviceFeed((async (command: string) => command === 'list_live_devices' ? [{ path_hex: 'ab12', module, serial_number: null }] : 'live') as never, listen, true, callback => callback());
      feed.setAutoDetect(true); feed.connect();
      await new Promise(resolve => setTimeout(resolve, 0));
      return feed;
    };
    const lighting = new LightingStateStore(null);
    lighting.record(['ab12'], () => ({ kind: 'led', leds: [{ colour: '#ff0000', effect: 1, brightness: 100 }, null, null] }));
    const knobsMarkup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={await make('m003k')} lighting={lighting} />);
    expect(knobsMarkup).toContain('module-art-inline');
    expect(knobsMarkup).toContain('color="#ff0000"');
    const faders = new LightingStateStore(null);
    faders.record(['ab12'], () => ({ kind: 'led', leds: [null, { colour: '#00ff00', effect: 5, brightness: 90 }, null, null, null] }));
    const fadersMarkup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={await make('m005s')} lighting={faders} />);
    expect(fadersMarkup).toContain('fill="#00ff00" opacity="1"');
    // An off LED keeps the art's resting look.
    expect(fadersMarkup).not.toContain('fill="#ff0000" opacity="1"');
  });

  it('shows only the connected base map when physical positions are available', async () => {
    const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Rig', bindings: [] });
    const state = new ProfileSession(() => new Promise(() => {}), profile, false).getState();
    const editor = { selection: { module: 'm003k', control: 0, gesture: 'clockwise', serial: '' }, faderPosition: 50, keypadPath: [keypadRoot], currentKeypadPage: state.draft.profile.keypad!.pages[0] } as unknown as DeviceEditor;
    const devices = [
      { path_hex: 'keys', module: 'm4315', serial_number: null, base_cell: { col: 0, row: -1 } },
      { path_hex: 'rollers', module: 'm032le', serial_number: null, base_cell: { col: 0, row: 3 } },
      { path_hex: 'dial', module: 'm131e', serial_number: null, base_cell: { col: 4, row: 0 } },
      { path_hex: 'knobs', module: 'm003k', serial_number: null, base_cell: { col: 4, row: 2 } },
    ];
    const feed = new DeviceFeed((async (command: string) => command === 'list_live_devices' ? devices : 'live') as never, async () => () => {}, true, callback => callback());
    feed.connect(); await new Promise(resolve => setTimeout(resolve, 0));
    const markup = renderToStaticMarkup(<RigPreview editor={editor} state={state} feed={feed} />);

    expect(markup).toContain('Connected base layout');
    expect(markup).toContain('MasterHUB base layout: 15-key IPS display at Top-left (1 row past the top edge), 2-roller scroll module at Bottom-left, Encoder IPS display at Top-right, 3-knob module at Bottom-right');
    expect(markup).toContain('aria-label="Select 3-knob module, Bottom-right on base" aria-pressed="true"');
    expect(markup).not.toContain('Add module');
    expect(markup).not.toContain('in rig');
    expect(markup).not.toContain('bound');
  });
});
