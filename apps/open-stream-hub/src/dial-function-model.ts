// Pure edits and checks for M131E dial functions (docs/masterhub-dial-functions.md).
// The Rust validator is authoritative; these checks mirror it so problems show
// inline before a save is refused.
import type { Action, Binding, DialBinding, DialFunction, Profile } from './contracts';

// DialFunction::MAX_FUNCTIONS and DialFunction::MAX_NAME_BYTES in crates/cooltek-sdk-service/src/dial.rs.
export const maxDialFunctions = 16;
export const maxDialFunctionNameBytes = 64;
// A short press switches functions; pressed and released fire on every click.
export const dialFunctionGestures: ReadonlyArray<DialBinding['gesture']> = ['right', 'left', 'long_press'];

const utf8 = new TextEncoder();

// Unicode General Category Cf, the same ranges as is_format_character in dial.rs.
const formatRanges: ReadonlyArray<readonly [number, number]> = [
  [0x00ad, 0x00ad], [0x0600, 0x0605], [0x061c, 0x061c], [0x06dd, 0x06dd], [0x070f, 0x070f], [0x0890, 0x0891],
  [0x08e2, 0x08e2], [0x180e, 0x180e], [0x200b, 0x200f], [0x202a, 0x202e], [0x2060, 0x2064], [0x2066, 0x206f],
  [0xfeff, 0xfeff], [0xfff9, 0xfffb], [0x110bd, 0x110bd], [0x110cd, 0x110cd], [0x13430, 0x1343f], [0x1bca0, 0x1bca3],
  [0x1d173, 0x1d17a], [0xe0001, 0xe0001], [0xe0020, 0xe007f],
];

// A lone UTF-16 surrogate cannot be encoded as UTF-8, so the service cannot store it.
const loneSurrogate = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;

function isControl(code: number) { return code <= 0x1f || (code >= 0x7f && code <= 0x9f); }
function isFormat(code: number) { return formatRanges.some(([low, high]) => code >= low && code <= high); }

export function hasDialFunctions(profile: Profile): profile is Profile & { dial_functions: DialFunction[] } {
  return Array.isArray(profile.dial_functions);
}

// The message for a function name the service would reject, or null. `index` is
// the function being named, so it is not compared with itself.
export function dialFunctionNameIssue(name: string, functions: readonly DialFunction[], index: number): string | null {
  if (!name.trim()) return 'Enter a function name.';
  if (loneSurrogate.test(name)) return 'Remove invalid characters.';
  if (name.trim() !== name) return 'Remove spaces from the start and end of the name.';
  if (utf8.encode(name).length > maxDialFunctionNameBytes) return `Use at most ${maxDialFunctionNameBytes} bytes.`;
  for (const char of name) {
    const code = char.codePointAt(0)!;
    if (isControl(code)) return 'Remove control characters such as line breaks and tabs.';
    if (isFormat(code)) return 'Remove invisible formatting characters such as zero-width spaces or direction marks.';
  }
  if (functions.some((other, position) => position !== index && other.name === name)) return `Another function is already named “${name}”.`;
  return null;
}

// The message for a serial the service would reject in a function binding, or null.
// An empty serial means "any device serial" and is never stored.
export function dialSerialIssue(serial: string): string | null {
  if (serial === '') return null;
  if (!serial.trim()) return 'A serial cannot be only spaces.';
  if (loneSurrogate.test(serial)) return 'Remove invalid characters.';
  if (utf8.encode(serial).length > 256) return 'Use at most 256 bytes.';
  if ([...serial].some(char => isControl(char.codePointAt(0)!))) return 'Remove control characters such as line breaks and tabs.';
  return null;
}

function nextName(functions: readonly DialFunction[]) {
  for (let number = functions.length + 1; ; number += 1) {
    const name = `Function ${number}`;
    if (!functions.some(item => item.name === name)) return name;
  }
}

function withFunctions(profile: Profile, functions: DialFunction[]): Profile {
  if (functions.length) return { ...profile, dial_functions: functions };
  // An empty array is rejected; no functions means the field is absent, as in profiles that never had them.
  const { dial_functions: _removed, ...rest } = profile;
  return rest;
}

export function addDialFunction(profile: Profile, name?: string): Profile {
  const functions = profile.dial_functions ?? [];
  if (functions.length >= maxDialFunctions) throw new Error(`A profile can have at most ${maxDialFunctions} dial functions.`);
  return withFunctions(profile, [...functions, { name: name ?? nextName(functions), bindings: [] }]);
}

// Keeps the typed text, even if invalid, so the field shows what was entered with an inline message.
export function renameDialFunction(profile: Profile, index: number, name: string): Profile {
  const functions = profile.dial_functions ?? [];
  if (!functions[index]) throw new Error('That dial function no longer exists.');
  return withFunctions(profile, functions.map((item, position) => position === index ? { ...item, name } : item));
}

export function moveDialFunction(profile: Profile, index: number, offset: -1 | 1): Profile {
  const functions = [...(profile.dial_functions ?? [])];
  const target = index + offset;
  if (!functions[index] || target < 0 || target >= functions.length) return profile;
  [functions[index], functions[target]] = [functions[target], functions[index]];
  return withFunctions(profile, functions);
}

// Deleting the last function removes `dial_functions`, restoring flat encoder bindings.
export function deleteDialFunction(profile: Profile, index: number): Profile {
  const functions = profile.dial_functions ?? [];
  if (!functions[index]) return profile;
  return withFunctions(profile, functions.filter((_, position) => position !== index));
}

const sameDialSelector = (binding: DialBinding, gesture: DialBinding['gesture'], serial: string) =>
  binding.gesture === gesture && (binding.serial_number ?? '') === serial;

export function dialBindingAt(profile: Profile, index: number, gesture: DialBinding['gesture'], serial: string): DialBinding | undefined {
  return profile.dial_functions?.[index]?.bindings.find(binding => sameDialSelector(binding, gesture, serial));
}

// Sets or (with a null action) removes the exact (gesture, serial) selector of one function.
export function editDialBinding(profile: Profile, index: number, gesture: DialBinding['gesture'], serial: string, action: Action | null): Profile {
  const functions = profile.dial_functions ?? [];
  const target = functions[index];
  if (!target) throw new Error('That dial function no longer exists.');
  if (!dialFunctionGestures.includes(gesture)) throw new Error('Dial functions bind only Right, Left and Long press.');
  const bindings = [...target.bindings];
  const position = bindings.findIndex(binding => sameDialSelector(binding, gesture, serial));
  if (!action) {
    if (position !== -1) bindings.splice(position, 1);
  } else {
    const binding: DialBinding = { gesture, ...(serial ? { serial_number: serial } : {}), action };
    if (position === -1) bindings.push(binding); else bindings[position] = binding;
  }
  return withFunctions(profile, functions.map((item, at) => at === index ? { ...item, bindings } : item));
}

export function removeDialBinding(profile: Profile, index: number, bindingIndex: number): Profile {
  const functions = profile.dial_functions ?? [];
  if (!functions[index]) return profile;
  return withFunctions(profile, functions.map((item, at) => at === index ? { ...item, bindings: item.bindings.filter((_, position) => position !== bindingIndex) } : item));
}

const isFlatDial = (binding: Binding) => binding.module === 'm131e';
const movable = (binding: Binding): binding is Binding & { gesture: DialBinding['gesture']; action: Action } =>
  isFlatDial(binding) && dialFunctionGestures.includes(binding.gesture as DialBinding['gesture']) && binding.action.type !== 'volume_from_position';

// What moving the flat encoder bindings into function `index` would do:
// `moved` are added to it, `conflicting` are dropped because the function
// already binds the same gesture and serial (the function's binding wins), and
// `dropped` are gestures or actions no function can hold. Without a function,
// every movable binding counts as moved.
export function flatDialBindings(profile: Profile, index?: number) {
  const flat = profile.bindings.filter(isFlatDial);
  const target = index === undefined ? undefined : profile.dial_functions?.[index];
  const taken = (binding: Binding & { gesture: DialBinding['gesture'] }) =>
    !!target?.bindings.some(existing => sameDialSelector(existing, binding.gesture, binding.serial_number ?? ''));
  const candidates = flat.filter(movable);
  return { moved: candidates.filter(binding => !taken(binding)), conflicting: candidates.filter(taken), dropped: flat.filter(binding => !movable(binding)) };
}

// Explicit user action: moves Right/Left/Long press flat encoder bindings into a
// function and removes every flat encoder binding. Bindings in
// flatDialBindings(...).conflicting and .dropped are lost; the UI names them first.
export function moveFlatDialBindings(profile: Profile, index: number): Profile {
  const functions = profile.dial_functions ?? [];
  const target = functions[index];
  if (!target) throw new Error('Add a dial function first.');
  const bindings = [...target.bindings];
  for (const binding of flatDialBindings(profile, index).moved) {
    const serial = binding.serial_number ?? '';
    bindings.push({ gesture: binding.gesture, ...(serial ? { serial_number: serial } : {}), action: binding.action });
  }
  return { ...profile, bindings: profile.bindings.filter(binding => !isFlatDial(binding)), dial_functions: functions.map((item, at) => at === index ? { ...item, bindings } : item) };
}
