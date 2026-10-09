// Generates src/key-icons.ts, the bundled key-icon catalog, from the lucide-react
// package already in node_modules (no network, no new runtime dependency). Run from
// apps/open-stream-hub: `node scripts/generate-key-icons.mjs`.
// The catalog is committed, so the app builds without re-running this and the UI
// never fetches icons from a CDN.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const iconsDirectory = join(root, 'node_modules', 'lucide-react', 'dist', 'esm', 'icons');
const output = join(root, 'src', 'key-icons.ts');

// The curated set, in display order. id must exist in the installed lucide-react;
// the script fails loudly otherwise. Keywords extend the search beyond the name.
const catalog = [
  ['volume-2', 'Volume', 'sound audio speaker loud louder'],
  ['volume-1', 'Volume low', 'sound audio speaker quiet'],
  ['volume-x', 'Muted', 'sound audio speaker silent mute'],
  ['mic', 'Microphone', 'record voice mic talk'],
  ['mic-off', 'Microphone off', 'record voice mic muted'],
  ['play', 'Play', 'media start music go'],
  ['pause', 'Pause', 'media stop hold music'],
  ['square', 'Stop', 'media end square halt'],
  ['skip-forward', 'Next track', 'media next skip forward'],
  ['skip-back', 'Previous track', 'media previous back skip'],
  ['repeat', 'Repeat', 'media loop again'],
  ['shuffle', 'Shuffle', 'media random mix'],
  ['music', 'Music', 'song audio note media'],
  ['headphones', 'Headphones', 'audio listen sound'],
  ['clapperboard', 'OBS scene', 'obs stream scene video'],
  ['video', 'Camera', 'obs webcam video record'],
  ['video-off', 'Camera off', 'obs webcam video muted'],
  ['circle', 'Record', 'obs record dot rec'],
  ['radio', 'Live', 'obs live stream broadcast on air'],
  ['film', 'Clip', 'obs replay clip video movie'],
  ['columns-2', 'Studio mode', 'obs studio split panes'],
  ['eye', 'Visible', 'show eye watch visible'],
  ['eye-off', 'Hidden', 'hide eye off invisible'],
  ['layers', 'Scene collection', 'obs collection stack layers'],
  ['app-window', 'Application', 'open app launch program'],
  ['globe', 'Website', 'url open browser web link http'],
  ['link', 'Link', 'url open web address'],
  ['keyboard', 'Shortcut', 'keyboard hotkey keys press'],
  ['type', 'Text', 'paste type text write'],
  ['list-ordered', 'Multi-action', 'multi steps sequence list'],
  ['arrow-left-right', 'Switch profile', 'profile switch next previous change'],
  ['timer', 'Timer', 'countdown stopwatch'],
  ['alarm-clock', 'Alarm', 'timer clock wake'],
  ['hourglass', 'Hourglass', 'timer wait time'],
  ['undo-2', 'Undo', 'system undo revert back'],
  ['redo-2', 'Redo', 'system redo again forward'],
  ['camera', 'Screenshot', 'system screenshot capture photo'],
  ['lightbulb', 'Light', 'light lamp brightness idea'],
  ['sun', 'Brightness up', 'sun light bright day'],
  ['moon', 'Dim', 'moon dark night'],
  ['zap', 'Boost', 'power zap energy fast'],
  ['power', 'Power', 'on off shutdown sleep'],
  ['wifi', 'Wi-Fi', 'wifi network wireless internet'],
  ['wifi-off', 'Wi-Fi off', 'wifi network off disconnected'],
  ['bluetooth', 'Bluetooth', 'bluetooth wireless pair'],
  ['battery-full', 'Battery', 'battery power charge'],
  ['bell', 'Notifications', 'bell alert ring'],
  ['bell-off', 'Notifications off', 'bell mute silent'],
  ['trash-2', 'Delete', 'trash remove bin'],
  ['save', 'Save', 'save store disk'],
  ['download', 'Download', 'download save transfer'],
  ['upload', 'Upload', 'upload send transfer'],
  ['refresh-cw', 'Refresh', 'refresh reload update sync'],
  ['settings', 'Settings', 'settings options gear configure'],
  ['home', 'Home', 'home main start house'],
  ['star', 'Star', 'star favourite favorite rate'],
  ['heart', 'Heart', 'heart like love favourite'],
  ['thumbs-up', 'Like', 'thumbs up like approve good'],
  ['lock', 'Locked', 'lock secure private'],
  ['lock-open', 'Unlocked', 'lock open unlocked'],
  ['search', 'Search', 'find look magnifier'],
  ['plus', 'Add', 'plus add new create'],
  ['minus', 'Remove', 'minus subtract less'],
  ['x', 'Close', 'close cancel stop'],
  ['check', 'Done', 'check done ok complete'],
  ['chevron-up', 'Up', 'chevron up arrow'],
  ['chevron-down', 'Down', 'chevron down arrow'],
  ['chevron-left', 'Left', 'chevron left arrow'],
  ['chevron-right', 'Right', 'chevron right arrow'],
  ['gamepad-2', 'Game', 'game controller play gamepad'],
  ['coffee', 'Coffee', 'coffee break cup cafe'],
  ['rocket', 'Launch', 'rocket launch start ship'],
  ['flame', 'Fire', 'flame fire hot'],
  ['snowflake', 'Cold', 'snowflake cold freeze winter'],
  ['palette', 'Colours', 'palette colour paint design'],
  ['scissors', 'Cut', 'scissors cut trim'],
  ['paperclip', 'Attach', 'paperclip attach file'],
  ['mail', 'Mail', 'mail email message inbox'],
  ['message-circle', 'Chat', 'chat message talk comment'],
  ['phone', 'Phone', 'phone call ring'],
  ['flag', 'Flag', 'flag mark report'],
  ['bookmark', 'Bookmark', 'bookmark save later'],
  ['tag', 'Tag', 'tag label category'],
  ['calendar', 'Calendar', 'calendar date event schedule'],
  ['party-popper', 'Celebrate', 'party celebrate confetti'],
  ['tv', 'Screen', 'tv display monitor'],
  ['speaker', 'Speaker', 'speaker audio output sound'],
  ['airplay', 'AirPlay', 'airplay stream cast screen'],
  ['cast', 'Cast', 'cast stream screen'],
  ['screen-share', 'Share screen', 'share screen present demo'],
  ['hand', 'Hand', 'hand wave hello'],
  ['flashlight', 'Torch', 'flashlight torch light'],
  ['fan', 'Fan', 'fan cool air spin'],
  ['droplet', 'Water', 'droplet water drip'],
  ['leaf', 'Plant', 'leaf plant nature green'],
  ['sparkles', 'Sparks', 'sparkles magic clean'],
  ['grid-3x3', 'Grid', 'grid layout apps'],
  ['folder', 'Folder', 'folder directory'],
  ['folder-open', 'Folder open', 'folder directory open'],
  ['file-text', 'Notes', 'file notes text document'],
  ['image', 'Image', 'image picture photo'],
  ['hard-drive', 'Drive', 'drive disk storage ssd'],
  ['cpu', 'Processor', 'cpu processor chip computer'],
  ['database', 'Database', 'database data server storage'],
  ['cloud', 'Cloud', 'cloud weather online'],
  ['server', 'Server', 'server host rack'],
  ['terminal', 'Terminal', 'terminal console shell command'],
  ['code', 'Code', 'code develop program script'],
  ['puzzle', 'Extension', 'puzzle extension plugin'],
  ['key', 'Key', 'key password access'],
  ['shield', 'Shield', 'shield secure protect safe'],
  ['triangle-alert', 'Warning', 'warning alert caution problem'],
  ['circle-help', 'Help', 'help question support info'],
  ['disc-2', 'Disc', 'disc vinyl music album'],
  ['wand-sparkles', 'Magic', 'wand magic spell'],
];

const version = readFileSync(join(root, 'node_modules', 'lucide-react', 'package.json'), 'utf8')
  .match(/"version": "([^"]+)"/)?.[1] ?? '';
const iconFiles = new Set(readdirSync(iconsDirectory));
const markupOf = nodes => nodes.map(([tag, attributes, children]) => {
  const attrs = Object.entries(attributes ?? {}).filter(([name]) => name !== 'key')
    .map(([name, value]) => `${name}="${String(value)}"`).join(' ');
  return `<${tag}${attrs ? ` ${attrs}` : ''}>${children ? markupOf(children) : ''}</${tag}>`;
}).join('');

const readIconNode = async id => {
  // Alias ids (e.g. trash-2) re-export another icon's data; follow one hop.
  let source = await import(pathToFileURL(join(iconsDirectory, `${id}.mjs`)).href);
  if (!source.__iconData) {
    const target = readFileSync(join(iconsDirectory, `${id}.mjs`), 'utf8')
      .match(/export \{ default \} from '\.\/([\w-]+)\.mjs'/)?.[1];
    if (!target) return null;
    source = await import(pathToFileURL(join(iconsDirectory, `${target}.mjs`)).href);
  }
  const node = source.__iconData?.node;
  return Array.isArray(node) && node.length > 0 ? node : null;
};

const entries = [];
for (const [id, name, keywords] of catalog) {
  if (!iconFiles.has(`${id}.mjs`)) throw new Error(`Icon "${id}" does not exist in lucide-react ${version}`);
  const node = await readIconNode(id);
  if (!node) throw new Error(`Icon "${id}" has no icon data`);
  const quote = text => text.replaceAll('\\', '\\\\').replaceAll("'", "\\'");
  entries.push(`  { id: '${id}', name: '${quote(name)}', keywords: '${quote(keywords)}', svg: '${quote(markupOf(node))}' },`);
}

writeFileSync(output, `// Bundled key icons, generated by scripts/generate-key-icons.mjs from the
// lucide-react package in node_modules (version ${version}). Committed so the app
// builds without the generator and renders fully offline: never fetch icon art
// from a network. Each svg is the inner markup of a 24x24 lucide icon; stroke
// colour is applied when rendering (currentColor in the UI, an explicit colour
// on the key canvas).

export interface KeyIcon { id: string; name: string; keywords: string; svg: string }

export const keyIcons: KeyIcon[] = [
${entries.join('\n')}
];

export const keyIconById = new Map(keyIcons.map(icon => [icon.id, icon]));
`);
console.log(`Wrote ${output} with ${entries.length} icons`);
