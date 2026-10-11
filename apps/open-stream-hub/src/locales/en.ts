// English: the source catalogue. Every other language has exactly these keys (see pl.ts), each message either text with
// {name} places or one text per plural category.
import type { Message } from '../i18n';

export const en = {
  // The left navigation.
  'nav.primary': 'Primary navigation',
  'nav.workspace': 'Workspace',
  'nav.artwork': 'Display artwork',
  'nav.lighting': 'Lighting',
  'nav.compatibility': 'Compatibility',
  'nav.guide': 'Getting started',
  'nav.about': 'About',
  'nav.settings': 'Settings',
  'nav.hint.artwork': 'Prepare images for module displays offline',
  'nav.hint.lighting': 'Dial backlight and colour, and knob, fader and base lighting',
  'nav.collapse': 'Collapse',
  'nav.collapseTip': 'Collapse sidebar',
  'nav.expandTip': 'Expand sidebar',

  // The modules drawer.
  'drawer.title': 'Modules',
  'drawer.show': 'Show the module list',
  'drawer.hide': 'Hide the module list',
  'drawer.connected': '{count} connected',
  'drawer.none': 'No module',
  'drawer.empty': 'No modules are connected.',
  'drawer.group': 'Module',
  'drawer.marked': 'Included in Whole kit',

  // The change-history list and its button.
  'history.button': 'Change history',
  'history.buttonTip': 'Change history: see and jump back through recent changes',
  'history.buttonTipEmpty': 'Change history (nothing to undo yet)',
  'history.tip': 'Every change to the open profile is listed here. Pick one to go back to just before it, or forward through it. {undoKey} undoes the last change and {redoKey} repeats it.',
  'history.redo': 'Redo',
  'history.undo': 'Undo',
  'history.redoList': 'Changes you can redo, nearest first',
  'history.undoList': 'Changes you can undo, latest first',
  'history.nothing': 'Nothing to undo yet.',

  // The header.
  'header.undo': 'Undo',
  'header.undoNamed': 'Undo: {change}',
  'header.redo': 'Redo',
  'header.redoNamed': 'Redo: {change}',
  'header.save': 'Save profile',
  'header.saving': 'Saving…',
  'header.manageProfiles': 'Manage profiles',
  'header.status.preview': 'Browser preview',
  'header.status.unavailable': 'Device status unavailable',
  'header.status.checking': 'Checking devices…',
  'header.status.none': 'No modules connected',
  'header.status.connected': { one: '{count} module connected', other: '{count} modules connected' },

  // Appearance names, and the Settings page.
  'theme.group': 'Appearance',
  'theme.system': 'System',
  'theme.light': 'Light',
  'theme.dark': 'Dark',
  'settings.title': 'Settings',
  'settings.intro': 'Language, appearance, notices, local profile storage, the display, OBS Studio and diagnostics.',
  'settings.appearance': 'Appearance',
  'settings.appearanceNote': 'System follows the macOS appearance setting. Your choice is remembered locally.',
  'settings.language': 'Language',
  'settings.languageNote': 'The language of the app. System default follows the language of your Mac when the app has it, and English otherwise. The change applies at once and is remembered locally; profiles, key labels and anything you typed stay as you wrote them.',
  'language.system': 'System default',
} satisfies Record<string, Message>;

export type MessageKey = keyof typeof en;
