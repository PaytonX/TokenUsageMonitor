// Generates a 1024x1024 PNG with a cyan-to-purple diagonal gradient + a
// centered "TM" monogram block. Used as the source for `tauri icon`.
// Pure Node + built-in zlib; no external deps.

import zlib from "node:zlib";
import { writeFileSync } from "node:fs";
import { Buffer } from "node:buffer";

const W = 1024;
const H = 1024;

function crc32(buf) {
  // Standard PNG CRC32. Pre-computed table for speed.
  let c = ~crc32.table;
  if (!crc32.table) {
    crc32.table = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let k = n;
      for (let i = 0; i < 8; i++) k = k & 1 ? 0xedb88320 ^ (k >>> 1) : k >>> 1;
      crc32.table[n] = k >>> 0;
    }
    c = ~0;
  }
  for (let i = 0; i < buf.length; i++) {
    c = crc32.table[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  }
  return (~c) >>> 0;
}
crc32.table = null;

function chunk(type, data) {
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([
    Buffer.from((data.length >>> 0).toString(16).padStart(8, "0").slice(-8), "hex"),
    typeBuf,
    data,
    crc,
  ]);
}

// Simple PNG encoder: 8-bit RGB (color type 2), no alpha.
// Each scanline prefixed with filter byte 0 (None).
function encodePng(width, height, rgbBuf) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type RGB
  ihdr[10] = 0; // compression
  ihdr[11] = 0; // filter
  ihdr[12] = 0; // interlace

  // Build raw scanlines with filter prefix.
  const rowLen = width * 3;
  const raw = Buffer.alloc((rowLen + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (rowLen + 1)] = 0; // filter None
    rgbBuf.copy(raw, y * (rowLen + 1) + 1, y * rowLen, (y + 1) * rowLen);
  }
  const idat = zlib.deflateSync(raw, { level: 9 });

  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// Cyan→Purple diagonal gradient.
function colorAt(x, y) {
  const t = (x + y) / (W + H - 2);
  const r = Math.round(34 + (167 - 34) * t);   // 22 -> a78bfa
  const g = Math.round(211 - (211 - 139) * t); // d3 -> 8b
  const b = Math.round(238 - (238 - 250) * t);  // ee -> fa
  return [r, g, b];
}

// Draw a rounded rectangle "TM" badge in the center for visual identity.
// Simple alpha-less version: darken pixels inside the badge area.
function inRoundedRect(px, py, x, y, w, h, r) {
  if (px < x || px >= x + w || py < y || py >= y + h) return false;
  if (px < x + r && py < y + r) {
    const dx = x + r - px, dy = y + r - py;
    return dx * dx + dy * dy <= r * r;
  }
  if (px >= x + w - r && py < y + r) {
    const dx = px - (x + w - r - 1), dy = y + r - py;
    return dx * dx + dy * dy <= r * r;
  }
  if (px < x + r && py >= y + h - r) {
    const dx = x + r - px, dy = py - (y + h - r - 1);
    return dx * dx + dy * dy <= r * r;
  }
  if (px >= x + w - r && py >= y + h - r) {
    const dx = px - (x + w - r - 1), dy = py - (y + h - r - 1);
    return dx * dx + dy * dy <= r * r;
  }
  return true;
}

const rgb = Buffer.alloc(W * H * 3);
for (let y = 0; y < H; y++) {
  for (let x = 0; x < W; x++) {
    const [r, g, b] = colorAt(x, y);
    const i = (y * W + x) * 3;
    // Draw a 560x560 rounded badge centered, slightly darker for contrast.
    const inBadge = inRoundedRect(x, y, (W - 560) / 2, (H - 560) / 2, 560, 560, 80);
    if (inBadge) {
      rgb[i] = Math.round(r * 0.35);
      rgb[i + 1] = Math.round(g * 0.35);
      rgb[i + 2] = Math.round(b * 0.35);
    } else {
      rgb[i] = r;
      rgb[i + 1] = g;
      rgb[i + 2] = b;
    }
  }
}

// Draw "TM" via crude 5x7 bitmap font, scaled up ~6x, two chars centered.
// Each glyph defined as rows of 1/0 bits.
const FONT = {
  T: [
    [1, 1, 1, 1, 1],
    [0, 0, 1, 0, 0],
    [0, 0, 1, 0, 0],
    [0, 0, 1, 0, 0],
    [0, 0, 1, 0, 0],
    [0, 0, 1, 0, 0],
    [0, 0, 1, 0, 0],
  ],
  M: [
    [1, 0, 0, 0, 1],
    [1, 1, 0, 1, 1],
    [1, 0, 1, 0, 1],
    [1, 0, 0, 0, 1],
    [1, 0, 0, 0, 1],
    [1, 0, 0, 0, 1],
    [1, 0, 0, 0, 1],
  ],
};
const SCALE = 14;
const GLYPH_W = 5;
const GLYPH_H = 7;
const GLYPH_GAP = 1;
const TM_W = (GLYPH_W * 2 + GLYPH_GAP) * SCALE;
const TM_H = GLYPH_H * SCALE;
const TM_X = Math.floor((W - TM_W) / 2);
const TM_Y = Math.floor((H - TM_H) / 2);

function drawGlyph(glyph, baseX, baseY) {
  for (let row = 0; row < glyph.length; row++) {
    for (let col = 0; col < glyph[row].length; col++) {
      if (!glyph[row][col]) continue;
      for (let dy = 0; dy < SCALE; dy++) {
        for (let dx = 0; dx < SCALE; dx++) {
          const px = baseX + col * SCALE + dx;
          const py = baseY + row * SCALE + dy;
          if (px < 0 || px >= W || py < 0 || py >= H) continue;
          const i = (py * W + px) * 3;
          rgb[i] = 0xf8;
          rgb[i + 1] = 0xfa;
          rgb[i + 2] = 0xfc;
        }
      }
    }
  }
}

drawGlyph(FONT.T, TM_X, TM_Y);
drawGlyph(FONT.M, TM_X + (GLYPH_W + GLYPH_GAP) * SCALE, TM_Y);

const png = encodePng(W, H, rgb);
writeFileSync(process.argv[2] ?? "icon-source.png", png);
console.log(`wrote ${(process.argv[2] ?? "icon-source.png")} (${png.length} bytes)`);
