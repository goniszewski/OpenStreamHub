import type { MessageKey } from '../locales/en';

export type DemoModule = 'm4315';

export type Demo = { id: string; name: MessageKey; description: MessageKey; module: DemoModule };

export const demoModuleName: Record<DemoModule, MessageKey> = { m4315: 'demos.module.keypad' };

export const demos: readonly Demo[] = [
  { id: 'tic-tac-toe', name: 'demos.ticTacToe.name', description: 'demos.ticTacToe.description', module: 'm4315' },
];

// A demo is available when a module of the kind it needs is connected.
export const demoAvailable = (demo: Demo, connected: readonly { module: string }[]): boolean => connected.some(device => device.module === demo.module);
