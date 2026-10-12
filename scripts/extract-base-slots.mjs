#!/usr/bin/env node
// Turns the slot geometry of the official MasterHUB app's base drawing (Baseboard.vue, 1.0.0.113) into our own
// slot table: for each module class, the base connector slot (the port chain below the base hub, digits joined) ->
// the cell its footprint starts at on our 6 x 4 landscape base, plus the rotation the official drawing gives it.
// Only positions and rotations are read; no CSS or art is copied. The official sources are not in this repo:
//
//   node scripts/extract-base-slots.mjs ~/Documents/masterhub-analysis/1.0.0.113/app-analysis/sources/src/components/Baseboard.vue
//
// Writes apps/open-stream-hub/src-tauri/data/base-slots.json. Check the result against docs/masterhub-hardware-results.md.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const source = process.argv[2];
if (!source) { console.error('usage: extract-base-slots.mjs <path to Baseboard.vue>'); process.exit(2); }

// The official drawing is a portrait 4 x 6 base: cells are 129 px apart, the first starts at 25 px, and the drawing
// is 824 px tall (elements anchored by `bottom` are measured from there). Our frame is the same base turned a
// quarter, 6 columns x 4 rows with the USB-C edges at the top and left; the mapping was checked against every
// arrangement recorded in the hardware results.
const pitch = 129, origin = 25, drawingHeight = 824, tolerance = 12;
// Footprints in cells (columns x rows) before any rotation, and the official class that draws each module.
const modules = {
  M4315: { cssClass: 'control-keypad', width: 4, height: 3 },
  M032LE: { cssClass: 'roller', width: 4, height: 1 },
  M005S: { cssClass: 'control-sliders5', width: 4, height: 2 },
  M131E: { cssClass: 'knobx1', width: 2, height: 2 },
  M003K: { cssClass: 'knob3', width: 2, height: 2 },
};

const css = readFileSync(source, 'utf8').replace(/^[\s\S]*?<style[^>]*>/, '').replace(/\/\*[\s\S]*?\*\//g, '');
const slots = {};
for (const [, selectors, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
  const props = Object.fromEntries([...body.matchAll(/([\w-]+)\s*:\s*([^;]+);/g)].map(m => [m[1], m[2].trim()]));
  for (const selector of selectors.split(',')) {
    const match = selector.trim().match(/^\.([\w-]+)\.pos(\d+)$/);
    if (!match) continue;
    const entry = (slots[match[1]] ??= {})[match[2]] ??= {};
    for (const key of ['top', 'left', 'bottom', 'transform']) if (key in props) entry[key] = props[key];
  }
}

const px = value => (value === undefined ? undefined : parseFloat(value));
const output = { source: 'official app 1.0.0.113, Baseboard.vue (geometry only)', frame: 'landscape 6 columns x 4 rows, USB-C edges at the top and left', modules: {} };
const dropped = [];
for (const [module, { cssClass, width, height }] of Object.entries(modules)) {
  output.modules[module] = {};
  for (const [slot, rule] of Object.entries(slots[cssClass] ?? {})) {
    const turn = rule.transform?.match(/rotate\((-?\d+)deg\)/);
    const rotation = (((turn ? parseInt(turn[1], 10) : 0) % 360) + 360) % 360;
    const sideways = rotation === 90 || rotation === 270;
    const boxWidth = width * pitch, boxHeight = height * pitch;
    // A rule that sets only one side relies on the class default, which is the first cell.
    const left = px(rule.left) ?? origin;
    const top = rule.top !== undefined ? px(rule.top) : rule.bottom !== undefined ? drawingHeight - px(rule.bottom) - boxHeight : origin;
    // A rotation turns the box about its centre, so the visible box swaps width and height.
    const centreX = left + boxWidth / 2, centreY = top + boxHeight / 2;
    const visibleWidth = sideways ? boxHeight : boxWidth, visibleHeight = sideways ? boxWidth : boxHeight;
    const column = (centreX - visibleWidth / 2 - origin) / pitch, row = (centreY - visibleHeight / 2 - origin) / pitch;
    if (Math.abs(column - Math.round(column)) * pitch > tolerance || Math.abs(row - Math.round(row)) * pitch > tolerance) {
      dropped.push(`${module} ${slot}: off the cell grid (${column.toFixed(2)}, ${row.toFixed(2)})`); continue;
    }
    const portraitColumn = Math.round(column), portraitRow = Math.round(row);
    const cellsWide = visibleWidth / pitch, cellsTall = visibleHeight / pitch;
    if (portraitColumn + cellsWide <= 0 || portraitColumn >= 4 || portraitRow + cellsTall <= 0 || portraitRow >= 6) {
      dropped.push(`${module} ${slot}: outside the base`); continue;
    }
    // Portrait -> landscape: the portrait bottom becomes the left edge.
    output.modules[module][slot] = { col: 6 - portraitRow - cellsTall, row: portraitColumn, official_rotation: rotation };
  }
}

const target = path.join(path.dirname(fileURLToPath(import.meta.url)), '../apps/open-stream-hub/src-tauri/data/base-slots.json');
writeFileSync(target, `${JSON.stringify(output, null, 1)}\n`);
for (const line of dropped) console.log(`dropped ${line}`);
console.log(`wrote ${Object.values(output.modules).reduce((n, m) => n + Object.keys(m).length, 0)} slots to ${target}`);
