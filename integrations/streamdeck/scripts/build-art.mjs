// Draws every image the plugin ships, from the Seam mark: two screen outlines
// with the amber seam between them. Run `pnpm art` after changing a glyph.
//
// A seam drawn solid and amber means the thing is on. A dashed muted seam means
// it's off, the same way the Displays map draws a border that isn't lined up.
// The titles in the manifest say the same in words, so colour is never alone.

import { Resvg } from "@resvg/resvg-js";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import url from "node:url";

const root = path.join(path.dirname(url.fileURLToPath(import.meta.url)), "..", "co.f3nn3x.seam.sdPlugin", "imgs");

const color = { page: "#202020", deep: "#1A1A1A", text: "#F3F3F3", muted: "#949494", seam: "#FFB020" };

/** Look of a glyph: the outline colour, and whether the seam is lit. */
const lit = { line: color.text, seam: color.seam, solid: true, width: 1 };
const idle = { line: color.muted, seam: color.muted, solid: false, width: 1 };
/** Action list and category icons are single-colour white. */
const white = { line: "#FFFFFF", seam: "#FFFFFF", solid: true, width: 1.6 };

const screen = (x, y, w, h, p) => {
  const half = p.width / 2;
  return `<rect x="${x + half}" y="${y + half}" width="${w - p.width}" height="${h - p.width}" rx="2" fill="none" stroke="${p.line}" stroke-width="${p.width}"/>`;
};

const seam = (x, y, h, p) =>
  p.solid
    ? `<rect x="${x}" y="${y}" width="2" height="${h}" fill="${p.seam}"/>`
    : `<line x1="${x + 1}" y1="${y}" x2="${x + 1}" y2="${y + h}" stroke="${p.seam}" stroke-width="2" stroke-dasharray="2 2"/>`;

// Glyphs are drawn in their own units, 32 wide, and placed by `place`.
const mark = (p) => ({ w: 32, h: 20, svg: screen(0, 2, 12, 16, p) + seam(14, 4, 12, p) + screen(18, 0, 14, 20, p) });

const lineup = (p) => {
  const base = mark(p);
  const guide = `<line x1="-3" y1="10" x2="35" y2="10" stroke="${p.line}" stroke-width="0.75" stroke-dasharray="1.5 1.5"/>`;
  return { ...base, svg: base.svg + guide };
};

const remap = (p) => {
  const base = mark(p);
  const pointer = `<path d="M9 8 V18.5 L11.4 16.2 L13 19.6 L14.8 18.8 L13.2 15.5 H16.4 Z" fill="${p.line}" stroke="${color.page}" stroke-width="0.8" stroke-linejoin="round"/>`;
  return { ...base, svg: base.svg + pointer };
};

const zones = (p) => ({
  w: 32,
  h: 20,
  svg:
    screen(0, 0, 32, 20, p) +
    seam(15, 1, 18, p) +
    `<line x1="17" y1="10" x2="31" y2="10" stroke="${p.line}" stroke-width="${Math.min(p.width, 1)}"/>`,
});

const zoneStep = (dir) => (p) => {
  const base = zones(p);
  const [tail, tip, back] = dir === "next" ? [10, 22, 19] : [22, 10, 13];
  const arrow =
    `<line x1="${tail}" y1="25" x2="${tip}" y2="25" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round"/>` +
    `<polyline points="${back},22.5 ${tip},25 ${back},27.5" fill="none" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round" stroke-linejoin="round"/>`;
  return { ...base, h: 28, svg: base.svg + arrow };
};

const arrowBelow = (p) =>
  `<line x1="8" y1="25" x2="24" y2="25" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round"/>` +
  `<polyline points="21,22.5 24,25 21,27.5" fill="none" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round" stroke-linejoin="round"/>`;

/** A window on the left screen, and the arrow it moves along. */
const windowMove = (p) => {
  const base = mark(p);
  const window = `<rect x="3" y="6" width="6" height="8" rx="1" fill="${p.line}"/>`;
  return { ...base, h: 28, svg: base.svg + window + arrowBelow(p) };
};

/** An outlined window on the left screen, and the filled one focus goes to. */
const windowFocus = (p) => {
  const base = mark(p);
  const from = `<rect x="3.5" y="6.5" width="5" height="7" rx="1" fill="none" stroke="${p.line}" stroke-width="${Math.min(p.width, 1)}"/>`;
  const to = `<rect x="22" y="5" width="6" height="10" rx="1" fill="${p.line}"/>`;
  return { ...base, h: 28, svg: base.svg + from + to + arrowBelow(p) };
};

/** A window on the right screen with a pin above it, the way it stays above the others. */
const pinTop = (p) => {
  const base = mark(p);
  const window = `<rect x="21" y="9" width="8" height="8" rx="1" fill="${p.line}"/>`;
  const pin =
    `<circle cx="25" cy="4" r="2" fill="${p.seam}"/>` +
    `<line x1="25" y1="6" x2="25" y2="9" stroke="${p.line}" stroke-width="${Math.min(p.width, 1)}"/>`;
  return { ...base, svg: base.svg + window + pin };
};

/** Two outlined windows, the pins taken off them. */
const release = (p) => {
  const base = mark(p);
  const thin = Math.min(p.width, 1);
  const windows =
    `<rect x="3.5" y="6.5" width="5" height="7" rx="1" fill="none" stroke="${p.line}" stroke-width="${thin}"/>` +
    `<rect x="21.5" y="6.5" width="7" height="8" rx="1" fill="none" stroke="${p.line}" stroke-width="${thin}"/>`;
  const arrow =
    `<line x1="16" y1="27" x2="16" y2="21" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round"/>` +
    `<polyline points="13.5,23.5 16,21 18.5,23.5" fill="none" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round" stroke-linejoin="round"/>`;
  return { ...base, h: 28, svg: base.svg + windows + arrow };
};

/** A dashed window where it drifted to, the filled one back in its place, and the way back. */
const snapBack = (p) => {
  const base = mark(p);
  const thin = Math.min(p.width, 1);
  const drifted = `<rect x="3.5" y="6.5" width="5" height="7" rx="1" fill="none" stroke="${p.line}" stroke-width="${thin}" stroke-dasharray="1.5 1.5"/>`;
  const placed = `<rect x="22" y="5" width="6" height="10" rx="1" fill="${p.line}"/>`;
  return { ...base, h: 28, svg: base.svg + drifted + placed + arrowBelow(p) };
};

/** An outlined window on the right screen, the filled one on the left it goes back to, and the arrow pointing back. */
const switchBack = (p) => {
  const base = mark(p);
  const thin = Math.min(p.width, 1);
  const from = `<rect x="22.5" y="6.5" width="5" height="7" rx="1" fill="none" stroke="${p.line}" stroke-width="${thin}"/>`;
  const to = `<rect x="3" y="5" width="6" height="10" rx="1" fill="${p.line}"/>`;
  const arrow =
    `<line x1="24" y1="25" x2="8" y2="25" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round"/>` +
    `<polyline points="11,22.5 8,25 11,27.5" fill="none" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round" stroke-linejoin="round"/>`;
  return { ...base, h: 28, svg: base.svg + from + to + arrow };
};

/** A window on the right screen, and the arrow down onto the line it's saved to. */
const quickSave = (p) => {
  const base = mark(p);
  const window = `<rect x="22" y="5" width="6" height="10" rx="1" fill="${p.line}"/>`;
  const arrow =
    `<line x1="16" y1="20.5" x2="16" y2="25" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round"/>` +
    `<polyline points="13.5,22.5 16,25 18.5,22.5" fill="none" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round" stroke-linejoin="round"/>` +
    `<line x1="10" y1="27.5" x2="22" y2="27.5" stroke="${p.line}" stroke-width="${p.width}" stroke-linecap="round"/>`;
  return { ...base, h: 28, svg: base.svg + window + arrow };
};

/** A dashed window on the left screen, minimized, and the filled one on the right that stays. */
const focusOnly = (p) => {
  const base = mark(p);
  const thin = Math.min(p.width, 1);
  const away = `<rect x="3.5" y="6.5" width="5" height="7" rx="1" fill="none" stroke="${p.line}" stroke-width="${thin}" stroke-dasharray="1.5 1.5"/>`;
  const stays = `<rect x="22" y="5" width="6" height="10" rx="1" fill="${p.line}"/>`;
  return { ...base, svg: base.svg + away + stays };
};

/** The left screen dimmed, and the filled window on the right left bright. */
const spotlight = (p) => {
  const base = mark(p);
  const dim = `<rect x="0.5" y="2.5" width="11" height="15" rx="2" fill="${p.line}" fill-opacity="0.35"/>`;
  const bright = `<rect x="22" y="5" width="6" height="10" rx="1" fill="${p.line}"/>`;
  return { ...base, svg: base.svg + dim + bright };
};

const place = (glyph, scale, cx, cy) =>
  `<g transform="translate(${cx - (glyph.w * scale) / 2} ${cy - (glyph.h * scale) / 2}) scale(${scale})">${glyph.svg}</g>`;

const wrap = (size, body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${body}</svg>\n`;

/** A key image: the glyph high on a page-grey square, leaving the lower part for the title. */
const key = (draw, p, extra = "") => {
  const glyph = draw(p);
  return wrap(144, `<rect width="144" height="144" fill="${color.page}"/>${place(glyph, 3.2, 72, 50)}${extra}`);
};

/** The small amber dot in a key's top-right corner: its workspace is in use but drifted. */
const driftDot = `<circle cx="124" cy="20" r="10" fill="${color.seam}"/>`;

/** A 20 px action list icon. */
const icon = (draw) => {
  const glyph = draw(white);
  return wrap(20, place(glyph, 0.5625, 10, 10));
};

const write = (file, data) => {
  const target = path.join(root, file);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(target, data);
};

const actions = {
  workspace: { draw: mark, states: ["idle", "lit"] },
  lineup: { draw: lineup, states: ["lit"] },
  remap: { draw: remap, states: ["idle", "lit"] },
  zones: { draw: zones, states: ["idle", "lit"] },
  "zone-next": { draw: zoneStep("next"), states: ["lit"] },
  "zone-prev": { draw: zoneStep("prev"), states: ["lit"] },
  desk: { draw: mark, states: ["lit"] },
  "window-move": { draw: windowMove, states: ["lit"] },
  "window-focus": { draw: windowFocus, states: ["lit"] },
  "pin-top": { draw: pinTop, states: ["lit"] },
  release: { draw: release, states: ["lit"] },
  "snap-back": { draw: snapBack, states: ["lit"] },
  "switch-back": { draw: switchBack, states: ["lit"] },
  "quick-save": { draw: quickSave, states: ["lit"] },
  "focus-only": { draw: focusOnly, states: ["lit"] },
  spotlight: { draw: spotlight, states: ["idle", "lit"] },
};

for (const [name, { draw, states }] of Object.entries(actions)) {
  write(`actions/${name}/icon.svg`, icon(draw));
  for (const state of states) {
    const file = states.length === 1 ? "key.svg" : `key-${state === "lit" ? "on" : "off"}.svg`;
    write(`actions/${name}/${file}`, key(draw, state === "lit" ? lit : idle));
  }
}

// The workspace key the plugin shows while its workspace has drifted.
write("actions/workspace/key-drift.svg", key(mark, lit, driftDot));

// The plugin icon is a PNG: the mark on a rounded square, like the app icon.
const pluginIcon = wrap(
  256,
  `<rect width="256" height="256" rx="56" fill="${color.deep}"/>${place(mark(lit), 6, 128, 128)}`,
);
for (const [file, size] of [
  ["plugin/icon.png", 256],
  ["plugin/icon@2x.png", 512],
]) {
  write(file, new Resvg(pluginIcon, { fitTo: { mode: "width", value: size } }).render().asPng());
}

write("plugin/category.svg", wrap(28, place(mark(white), 0.75, 14, 14)));
