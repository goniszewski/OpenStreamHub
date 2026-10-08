import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { ProfileSession } from './profile-session';
import { RigPreview } from './RigPreview';
import { DeviceFeed } from './device-feed';
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
    expect(markup).toContain('Volume +5');
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
});
