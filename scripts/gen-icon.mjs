// 生成 1024x1024 应用图标（纯 Node 实现，无第三方依赖）
// 输出 scripts/icon-source.png，供 `tauri icon` 生成各平台尺寸
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const W = 1024;
const H = 1024;
const px = new Uint8Array(W * H * 4);

const lerp = (a, b, t) => a + (b - a) * t;

function insideRoundedRect(x, y, x0, y0, x1, y1, r) {
  const cx = Math.max(x0 + r, Math.min(x, x1 - r));
  const cy = Math.max(y0 + r, Math.min(y, y1 - r));
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= r * r;
}

function insideTriangle(px_, py_, ax, ay, bx, by, cx, cy) {
  const s1 = (bx - ax) * (py_ - ay) - (by - ay) * (px_ - ax);
  const s2 = (cx - bx) * (py_ - by) - (cy - by) * (px_ - bx);
  const s3 = (ax - cx) * (py_ - cy) - (ay - cy) * (px_ - cx);
  const hasNeg = s1 < 0 || s2 < 0 || s3 < 0;
  const hasPos = s1 > 0 || s2 > 0 || s3 > 0;
  return !(hasNeg && hasPos);
}

for (let y = 0; y < H; y++) {
  for (let x = 0; x < W; x++) {
    const t = y / H;
    // 背景：垂直渐变 #0B0F1A -> #111827
    let r = lerp(11, 17, t);
    let g = lerp(15, 24, t);
    let b = lerp(26, 39, t);
    let a = 255;

    if (insideRoundedRect(x, y, 28, 28, W - 28, H - 28, 210)) {
      // 顶部青色辉光
      const dx1 = x - W * 0.34;
      const dy1 = y - H * 0.28;
      const d1 = Math.sqrt(dx1 * dx1 + dy1 * dy1) / 700;
      const glowCyan = Math.max(0, 1 - d1);
      r += (34 - r) * glowCyan * 0.55;
      g += (211 - g) * glowCyan * 0.55;
      b += (238 - b) * glowCyan * 0.55;

      // 底部蓝色辉光
      const dx2 = x - W * 0.78;
      const dy2 = y - H * 0.85;
      const d2 = Math.sqrt(dx2 * dx2 + dy2 * dy2) / 620;
      const glowBlue = Math.max(0, 1 - d2);
      r += (59 - r) * glowBlue * 0.5;
      g += (130 - g) * glowBlue * 0.5;
      b += (246 - b) * glowBlue * 0.5;

      // 中心终端 ">" 箭头
      const tipX = W * 0.40, tipY = H * 0.50;
      const armX = W * 0.74, armTop = H * 0.31, armBot = H * 0.69;
      if (insideTriangle(x, y, tipX, tipY, armX, armTop, armX, armBot)) {
        const dist = Math.abs(x - tipX) / (armX - tipX);
        r = lerp(34, 59, dist);
        g = lerp(211, 130, dist);
        b = lerp(238, 246, dist);
        a = 255;
      }

      // 光标竖条
      const curX0 = W * 0.785, curX1 = W * 0.785 + 52, curY0 = H * 0.40, curY1 = H * 0.60;
      if (x >= curX0 && x <= curX1 && y >= curY0 && y <= curY1) {
        r = 249; g = 250; b = 251; a = 255;
      }
    } else {
      a = 0; // 圆角外透明
    }

    const i = (y * W + x) * 4;
    px[i] = r;
    px[i + 1] = g;
    px[i + 2] = b;
    px[i + 3] = a;
  }
}

// ---- PNG 编码 ----
const CRC_TABLE = new Int32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  CRC_TABLE[n] = c;
}
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const t = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
}
function pngEncode() {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(W, 0);
  ihdr.writeUInt32BE(H, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc((W * 4 + 1) * H);
  for (let y = 0; y < H; y++) {
    raw[y * (W * 4 + 1)] = 0; // filter none
    Buffer.from(px.buffer, y * W * 4, W * 4).copy(raw, y * (W * 4 + 1) + 1);
  }
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const out = join(__dirname, "icon-source.png");
writeFileSync(out, pngEncode());
console.log(`icon written: ${out}`);
