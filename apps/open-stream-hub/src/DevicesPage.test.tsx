import { beforeEach, describe, expect, it } from 'vitest';
import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { DeviceFeed } from './device-feed';
import { DeviceWorkspace, HeroImage, KnobOrientationControl, ModuleCatalog } from './DevicesPage';
import { Inspector } from './Inspector';
import { CompatibilityPage } from './InfoPages';
import { keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { knobOrientationStore } from './knob-orientation';
import { ProfileSession } from './profile-session';
import { moduleNote, modules } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

const profile = upgradeKeypadProfile({ schema_version: 1, name: 'Knobs', bindings: [] });
const state = new ProfileSession(() => new Promise(() => {}), profile, false).getState();
const editor = {
  selectedModule: 'm003k', selected: modules.find(module => module.id === 'm003k')!,
  selection: { module: 'm003k', control: 0, gesture: 'clockwise', serial: '' }, serialScope: 'any', faderPosition: 50,
  keypadPath: [keypadRoot], keypadPages: state.draft.profile.keypad!.pages, currentKeypadPage: state.draft.profile.keypad!.pages[0],
  isKeypad: false, binding: undefined,
} as unknown as DeviceEditor;
const render = () => renderToStaticMarkup(<DeviceWorkspace editor={editor} state={state} desktop={false} disabled={false} setPage={() => {}} />);

type Props = { onClick?: () => void; children?: ReactNode; 'aria-label'?: string };
const find = (node: ReactNode, match: (element: ReactElement<Props>) => boolean): ReactElement<Props> | undefined => {
  if (Array.isArray(node)) return node.map(child => find(child, match)).find(Boolean);
  if (!isValidElement<Props>(node)) return undefined;
  return match(node) ? node : find(node.props.children, match);
};

// The test env has no DOM, so this takes the element tree DeviceWorkspace itself returns, expands the
// orientation control it rendered (a plain function without hooks) and calls the real button's onClick.
function clickRotate() {
  let tree: ReactNode = null;
  function Capture() { tree = DeviceWorkspace({ editor, state, desktop: false, disabled: false, setPage: () => {} }); return null; }
  renderToStaticMarkup(<Capture />);
  const control = find(tree, element => element.type === KnobOrientationControl) as unknown as ReactElement<Parameters<typeof KnobOrientationControl>[0]>;
  const button = find(KnobOrientationControl(control.props), element => element.type === 'button' && element.props['aria-label'] === 'Rotate module 90 degrees clockwise');
  button!.props.onClick!();
}

const areaStyle = (markup: string, label: string) => {
  const tag = markup.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0] ?? '';
  const style = tag.match(/style="left:([\d.]+)%;top:([\d.]+)%/);
  return style ? { left: Number(style[1]), top: Number(style[2]) } : undefined;
};

describe('3-knob module orientation', () => {
  beforeEach(() => knobOrientationStore.set('single-top'));

  it('rotates the hero and rig art through all four orientations and wraps', () => {
    const steps: { label: string; rotation: number; single: { left: number; top: number }; first: { left: number; top: number } }[] = [
      { label: 'Single knob on top', rotation: 0, single: { left: 39, top: 22 }, first: { left: 22, top: 55 } },
      { label: 'Single knob on the right', rotation: 90, single: { left: 56, top: 39 }, first: { left: 23, top: 22 } },
      { label: 'Single knob at the bottom', rotation: 180, single: { left: 39, top: 56 }, first: { left: 56, top: 23 } },
      { label: 'Single knob on the left', rotation: 270, single: { left: 22, top: 39 }, first: { left: 55, top: 56 } },
      { label: 'Single knob on top', rotation: 0, single: { left: 39, top: 22 }, first: { left: 22, top: 55 } },
    ];
    steps.forEach((step, index) => {
      if (index) clickRotate();
      const markup = render();
      expect(markup).toContain(`aria-live="polite">${step.label}</span>`);
      expect(markup).toContain('aria-label="Rotate module 90 degrees clockwise"');
      // Both the hero and the rig tile use the top-down art with the same rotation.
      const images = markup.match(/<img src="\/modules\/knob-top\.svg"[^>]*>/g) ?? [];
      expect(images).toHaveLength(2);
      for (const image of images) expect(image).toContain(`transform:rotate(${step.rotation}deg)`);
      expect(markup).not.toContain('/modules/knob.png" alt="" draggable');
      for (const where of ['on module image', 'in rig']) {
        expect(areaStyle(markup, `Select Knob 3 ${where}`)).toEqual(step.single);
        expect(areaStyle(markup, `Select Knob 1 ${where}`)).toEqual(step.first);
        expect(areaStyle(markup, `Select Knob 2 ${where}`)).toBeDefined();
      }
      // Hit areas and their contents are not rotated.
      expect(markup).not.toMatch(/<button[^>]*image-control[^>]*transform/);
    });
  });

  it('keeps the other modules on their photos without the orientation control', () => {
    const faders = { ...editor, selectedModule: 'm005s', selected: modules.find(module => module.id === 'm005s')!, selection: { module: 'm005s', control: 0, gesture: 'moved', serial: '' } } as unknown as DeviceEditor;
    const markup = renderToStaticMarkup(<DeviceWorkspace editor={faders} state={state} desktop={false} disabled={false} setPage={() => {}} />);
    expect(markup).toContain('src="/modules/faders.png"');
    expect(markup).not.toContain('Rotate module');
  });

  it('shows the canonical top-down art, unrotated, in the catalog and the compatibility table', () => {
    knobOrientationStore.set('single-left');
    const catalog = renderToStaticMarkup(<ModuleCatalog editor={editor} />);
    expect(catalog).toContain('src="/modules/knob-top.svg"');
    expect(catalog).not.toContain('knob.png');
    expect(catalog).not.toMatch(/knob-top\.svg"[^>]*transform/);
    const table = renderToStaticMarkup(<CompatibilityPage />);
    expect(table).toMatch(/<img src="\/modules\/knob-top\.svg" alt=""\/>/);
    expect(table).not.toContain('knob.png');
  });
});

describe('hero live input', () => {
  beforeEach(() => knobOrientationStore.set('single-top'));

  async function liveFeed() {
    const handlers: Record<string, (event: { payload: unknown }) => void> = {};
    const feed = new DeviceFeed((async () => []) as never, async (event, handler) => { handlers[event] = handler; return () => {}; }, true, callback => callback());
    feed.connect(); await new Promise(resolve => setTimeout(resolve, 0));
    handlers['device://status']({ payload: { feed_id: 'f', state: 'running', source: 'demo', module: null } });
    const send = (module: string, control: number, gesture: string, value?: unknown) => handlers['device://input']({ payload: { feed_id: 'f', module, serial_number: null, event: { timestamp_micros: 1, control, gesture, value } } });
    return { feed, send };
  }
  const forModule = (id: string) => ({ ...editor, selectedModule: id, selected: modules.find(module => module.id === id)!, selection: { module: id, control: 0, gesture: 'pressed', serial: '' } } as unknown as DeviceEditor);
  const hero = (markup: string, name: string) => {
    const start = markup.indexOf(`aria-label="${name} image controls"`);
    expect(start).toBeGreaterThan(-1);
    return markup.slice(start, markup.indexOf('hero-note', start));
  };
  const heroButton = (markup: string, label: string) => markup.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>.*?</button>`))?.[0] ?? '';

  it('shows live rotation and presses of the selected 3-knob module on the hero', async () => {
    const { feed, send } = await liveFeed();
    const quiet = hero(renderToStaticMarkup(<DeviceWorkspace editor={editor} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), '3x Knob');
    expect(quiet).not.toContain('live-pointer');
    expect(quiet).not.toContain(' pressed');
    send('m003k', 2, 'clockwise'); send('m003k', 2, 'clockwise'); send('m003k', 0, 'pressed');
    const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={editor} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), '3x Knob');
    const knob3 = heroButton(markup, 'Select Knob 3 on module image');
    expect(knob3).toContain('live-pointer');
    expect(knob3).toContain('rotate(30deg)');
    // The knobs' needle keeps its full length; only the dial's is fitted inside its glass.
    expect(knob3).not.toContain('live-pointer screen');
    expect(knob3).not.toMatch(/class="[^"]*pressed/);
    expect(heroButton(markup, 'Select Knob 1 on module image')).toMatch(/class="image-control round selected pressed"/);
    // The needle sits inside an unrotated hit area, positioned by the computed percentages.
    expect(knob3).toContain('style="left:39%;top:22%');
  });

  it('shows the live encoder needle and press on the hero without making it a control', async () => {
    const { feed, send } = await liveFeed();
    send('m131e', 0, 'right'); send('m131e', 0, 'pressed');
    const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={forModule('m131e')} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), 'Encoder with IPS Display');
    expect(markup).toMatch(/<span class="image-control round live-overlay pressed" aria-hidden="true"[^>]*><span class="live-pointer screen" style="transform:rotate\(15deg\)"/);
    expect(markup).not.toContain('<button');
  });

  it('highlights live key presses on the 15-key hero', async () => {
    const { feed, send } = await liveFeed();
    send('m4315', 4, 'pressed');
    const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={{ ...forModule('m4315'), isKeypad: true } as DeviceEditor} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), '15x Key IPS Display');
    expect(heroButton(markup, 'Select Key 5 on module image')).toMatch(/class="image-control key-display pressed"/);
  });

  it('does not mark the 3-knob hero for live input from another module type', async () => {
    const { feed, send } = await liveFeed();
    send('m4315', 0, 'pressed'); send('m4315', 2, 'pressed'); send('m131e', 0, 'right');
    const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={editor} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), '3x Knob');
    expect(markup).not.toContain('live-pointer');
    expect(markup).not.toMatch(/class="[^"]*pressed/);
  });

  it('shows a live roller pulse on the hero', async () => {
    const { feed, send } = await liveFeed();
    send('m032le', 1, 'up');
    const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={forModule('m032le')} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), '2x Roller Scroll');
    const roller = markup.match(/<button[^>]*aria-label="Select [^"]+ on module image"[^>]*>.*?<\/button>/g) ?? [];
    expect(roller).toHaveLength(2);
    expect(roller.filter(button => button.includes('live-pulse up'))).toHaveLength(1);
    expect(roller.find(button => button.includes('live-pulse up'))).toContain('lucide-chevron-up');
  });

  it('shows the live fader thumb on the hero only where a live value exists', async () => {
    const { feed, send } = await liveFeed();
    send('m005s', 3, 'position_changed', { type: 'position', raw: 30, maximum: 100 });
    const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={forModule('m005s')} state={state} feed={feed} desktop disabled={false} setPage={() => {}} />), '5x Fader Precision');
    const faders = markup.match(/<button[^>]*aria-label="Select [^"]+ on module image"[^>]*>.*?<\/button>/g) ?? [];
    expect(faders).toHaveLength(5);
    expect(faders.filter(button => button.includes('fader-thumb'))).toHaveLength(1);
    expect(faders[3]).toMatch(/class="image-control live"/);
    expect(faders[3]).toContain('<span class="fader-thumb" style="bottom:30%"></span>');
  });

  it('keeps the feed subscription out of the surrounding workspace', () => {
    // Only components that read the feed store re-render on a feed event. DeviceWorkspace's own body
    // must not read it; the hero image (and the rig and status line, as before) are separate children.
    let reads = 0;
    const real = new DeviceFeed((async () => []) as never, async () => () => {}, true, callback => callback());
    const feed = { subscribe: real.subscribe, getState: () => { reads++; return real.getState(); } } as unknown as DeviceFeed;
    let tree: ReactNode = null;
    function Workspace() { tree = DeviceWorkspace({ editor, state, feed, desktop: true, disabled: false, setPage: () => {} }); return null; }
    renderToStaticMarkup(<Workspace />);
    expect(reads).toBe(0);
    const heroElement = find(tree, element => element.type === HeroImage) as unknown as ReactElement<{ feed?: DeviceFeed }>;
    expect(heroElement.props.feed).toBe(feed);
    renderToStaticMarkup(<HeroImage editor={editor} profile={state.draft.profile} feed={feed} knobOrientation="single-top" />);
    expect(reads).toBeGreaterThan(0);
  });

  it('changes nothing on the hero without a feed', () => {
    for (const id of ['m003k', 'm131e']) {
      const markup = hero(renderToStaticMarkup(<DeviceWorkspace editor={forModule(id)} state={state} desktop={false} disabled={false} setPage={() => {}} />), modules.find(module => module.id === id)!.name);
      expect(markup).not.toContain('live-pointer');
      expect(markup).not.toContain('live-pulse');
      expect(markup).not.toMatch(/class="[^"]*pressed/);
    }
  });
});

describe('module catalog labels', () => {  it('marks connected modules and leaves the rest neutral, with no planned or pending labels', async () => {
    const feed = new DeviceFeed((async () => []) as never, async () => () => {}, true, callback => callback());
    feed.connect(); await new Promise(resolve => setTimeout(resolve, 0));
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [{ path_hex: 'a1', module: 'm003k', serial_number: null }] };
    const markup = renderToStaticMarkup(<ModuleCatalog editor={editor} feed={feed} />);
    expect(markup).toContain('<small>Connected</small>');
    expect(markup).toContain('<small>Not connected</small>');
    expect(markup).not.toContain('Planned');
    expect(markup).not.toContain('pending');
    const quiet = renderToStaticMarkup(<ModuleCatalog editor={editor} />);
    expect(quiet.match(/<small>Not connected<\/small>/g)).toHaveLength(modules.length);
  });
});

describe('3-knob module wording', () => {
  it('states the verified knob order and the verified input', () => {
    expect(moduleNote('m003k')).toBe('Knob order verified on hardware: with the single knob on top, Knob 3 is on top, Knob 1 bottom-left and Knob 2 bottom-right.');
    expect(moduleNote('m005s')).toBe('Control positions will be checked with the device.');
    const rows = renderToStaticMarkup(<CompatibilityPage />).match(/<tr>.*?<\/tr>/g) ?? [];
    const knobs = rows.find(row => row.includes('3x Knob'));
    expect(knobs).toContain('<td>Input verified (own mode); the input-only run that starts by itself is not yet tested</td>');
    expect(knobs).toContain('<td>None; input only</td>');
    // The 15-key module and the dial: what one module each showed on hardware, nothing more.
    expect(rows.filter(row => row.includes('<td>Input and key images verified on one module (software mode)</td>'))).toHaveLength(1);
    expect(rows.filter(row => row.includes('<td>Input and dial faces verified on one module (software mode)</td>'))).toHaveLength(1);
    expect(rows.filter(row => row.includes('<td>Unverified</td>'))).toHaveLength(modules.length - 3);
  });
});

describe('workspace without the simulation panel', () => {
  it('leaves testing to the inspector: no Simulation section or per-gesture buttons remain', () => {
    const markup = render();
    expect(markup).not.toContain('Simulation');
    expect(markup).not.toContain('event-buttons');
    expect(markup).toContain('Run profile');
  });
});

describe('inspector test button and one-line result', () => {
  const session = new ProfileSession(() => new Promise(() => {}), profile, false);
  const inspector = (editorLike: Record<string, unknown>, sessionState = session.getState()) =>
    renderToStaticMarkup(<Inspector editor={editorLike as unknown as DeviceEditor} session={session} state={sessionState} disabled={false} desktop />);

  it('offers a compact test with its one-line result', () => {
    const markup = inspector(editor);
    expect(markup).toContain('>Test gesture</button>');
    expect(markup).toContain('Run a test to preview what the binding would do.');
    expect(markup).toContain('Tests never execute anything');
    const failed = { ...session.getState(), preview: { status: 'failed', error: 'No action is bound.' } as const };
    const failedMarkup = inspector(editor, failed);
    expect(failedMarkup).toContain('role="alert"');
    expect(failedMarkup).toContain('No action is bound.');
  });

  it('gives faders a position control and a test for the chosen position', () => {
    const faders = { ...editor, selectedModule: 'm005s', selected: modules.find(module => module.id === 'm005s')!, selection: { module: 'm005s', control: 0, gesture: 'moved', serial: '' }, faderPosition: 40 } as unknown as DeviceEditor;
    const markup = inspector(faders);
    expect(markup).toContain('id="fader-position"');
    expect(markup).toContain('value="40"');
    expect(markup).toContain('>Test position</button>');
  });

  it('says the desktop app is needed to preview in the browser', () => {
    const markup = renderToStaticMarkup(<Inspector editor={editor as unknown as DeviceEditor} session={session} state={session.getState()} disabled={false} />);
    expect(markup).toContain('Launch the desktop app to preview bindings.');
  });
});
