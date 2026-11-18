import type { MessageKey } from '../locales/en';

export type DemoModule = 'm4315' | 'm131e' | 'm003k' | 'm005s' | 'm032le';

// A demo names every module it needs; each shows as a tag on its card.
// `turns` lists modules a demo also uses when one is connected (the first that is), though it does not need them.
export type Demo = { id: string; name: MessageKey; description: MessageKey; modules: readonly DemoModule[]; turns?: readonly DemoModule[] };

export const demoModuleName: Record<DemoModule, MessageKey> = {
  m4315: 'demos.module.keypad',
  m131e: 'demos.module.dial',
  m003k: 'demos.module.knobs',
  m005s: 'demos.module.faders',
  m032le: 'demos.module.rollers',
};

export const demos: readonly Demo[] = [
  { id: 'tic-tac-toe', name: 'demos.ticTacToe.name', description: 'demos.ticTacToe.description', modules: ['m4315'] },
  { id: 'twofold', name: 'demos.twofold.name', description: 'demos.twofold.description', modules: ['m4315'], turns: ['m032le', 'm003k'] },
  { id: 'whack-a-mole', name: 'demos.whack.name', description: 'demos.whack.description', modules: ['m4315'] },
  { id: 'display-speed', name: 'demos.speed.name', description: 'demos.speed.description', modules: ['m4315'] },
];

export const modulePresent = (module: DemoModule, connected: readonly { module: string }[]): boolean => connected.some(device => device.module === module);

// A demo is available when every module it needs is connected.
export const demoAvailable = (demo: Demo, connected: readonly { module: string }[]): boolean => demo.modules.every(module => modulePresent(module, connected));

// The modules a demo needs that are not connected.
export const demoMissing = (demo: Demo, connected: readonly { module: string }[]): DemoModule[] => demo.modules.filter(module => !modulePresent(module, connected));
