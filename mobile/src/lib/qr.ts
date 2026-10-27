// Dependency-free QR encoder for short sign-in links and device codes: byte mode, error
// correction level M, versions 1-40 (up to 2331 bytes). Longer input throws instead of truncating.

type Matrix = boolean[][];

interface VersionSpec {
  /** Error-correction codewords per block. */
  ec: number;
  /** [block count, data codewords per block] groups. */
  groups: [number, number][];
}

// Level M, ISO/IEC 18004 table 9: [ec codewords per block, [blocks, data codewords] groups].
const SPEC_ROWS: [number, [number, number][]][] = [
  [10, [[1, 16]]], [16, [[1, 28]]], [26, [[1, 44]]], [18, [[2, 32]]], [24, [[2, 43]]],
  [16, [[4, 27]]], [18, [[4, 31]]], [22, [[2, 38], [2, 39]]], [22, [[3, 36], [2, 37]]],
  [26, [[4, 43], [1, 44]]], [30, [[1, 50], [4, 51]]], [22, [[6, 36], [2, 37]]],
  [22, [[8, 37], [1, 38]]], [24, [[4, 40], [5, 41]]], [24, [[5, 41], [5, 42]]],
  [28, [[7, 45], [3, 46]]], [28, [[10, 46], [1, 47]]], [26, [[9, 43], [4, 44]]],
  [26, [[3, 44], [11, 45]]], [26, [[3, 41], [13, 42]]], [26, [[17, 42]]], [28, [[17, 46]]],
  [28, [[4, 47], [14, 48]]], [28, [[6, 45], [14, 46]]], [28, [[8, 47], [13, 48]]],
  [28, [[19, 46], [4, 47]]], [28, [[22, 45], [3, 46]]], [26, [[3, 45], [23, 46]]],
  [28, [[21, 45], [7, 46]]], [28, [[19, 47], [10, 48]]], [28, [[2, 46], [29, 47]]],
  [28, [[10, 46], [23, 47]]], [28, [[14, 46], [21, 47]]], [28, [[14, 46], [23, 47]]],
  [28, [[12, 47], [26, 48]]], [28, [[6, 47], [34, 48]]], [28, [[29, 46], [14, 47]]],
  [28, [[13, 46], [32, 47]]], [28, [[40, 47], [7, 48]]], [28, [[18, 47], [31, 48]]],
];

const SPECS: VersionSpec[] = SPEC_ROWS.map(([ec, groups]) => ({ ec, groups }));

/** Alignment pattern centres for `version` (empty for version 1). */
function alignmentPositions(version: number): number[] {
  if (version === 1) return [];
  const count = Math.floor(version / 7) + 2;
  const size = 17 + 4 * version;
  const step = version === 32 ? 26 : Math.ceil((version * 4 + 4) / (count * 2 - 2)) * 2;
  const out = [6];
  for (let pos = size - 7; out.length < count; pos -= step) out.splice(1, 0, pos);
  return out;
}

const EXP = new Array<number>(512);
const LOG = new Array<number>(256);
(() => {
  let x = 1;
  for (let i = 0; i < 255; i++) {
    EXP[i] = x;
    LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255];
})();

function gfMul(a: number, b: number): number {
  return a === 0 || b === 0 ? 0 : EXP[LOG[a] + LOG[b]];
}

function rsRemainder(data: number[], ecLen: number): number[] {
  let gen = [1];
  for (let i = 0; i < ecLen; i++) {
    const next = new Array<number>(gen.length + 1).fill(0);
    for (let j = 0; j < gen.length; j++) {
      next[j] ^= gen[j];
      next[j + 1] ^= gfMul(gen[j], EXP[i]);
    }
    gen = next;
  }
  const rem = new Array<number>(ecLen).fill(0);
  for (const byte of data) {
    const factor = byte ^ rem[0];
    rem.shift();
    rem.push(0);
    for (let i = 0; i < ecLen; i++) rem[i] ^= gfMul(gen[i + 1], factor);
  }
  return rem;
}

function dataCapacity(spec: VersionSpec): number {
  return spec.groups.reduce((sum, [n, d]) => sum + n * d, 0);
}

function encodeCodewords(bytes: number[], version: number): number[] {
  const spec = SPECS[version - 1];
  const bits: number[] = [];
  const push = (value: number, count: number) => {
    for (let i = count - 1; i >= 0; i--) bits.push((value >>> i) & 1);
  };
  push(0b0100, 4);
  push(bytes.length, version >= 10 ? 16 : 8);
  for (const b of bytes) push(b, 8);
  const capBits = dataCapacity(spec) * 8;
  push(0, Math.min(4, capBits - bits.length));
  while (bits.length % 8) bits.push(0);
  const data: number[] = [];
  for (let i = 0; i < bits.length; i += 8) {
    data.push(parseInt(bits.slice(i, i + 8).join(""), 2));
  }
  for (let pad = 0xec; data.length < dataCapacity(spec); pad ^= 0xec ^ 0x11) data.push(pad);

  const dataBlocks: number[][] = [];
  let at = 0;
  for (const [count, size] of spec.groups) {
    for (let i = 0; i < count; i++) {
      dataBlocks.push(data.slice(at, at + size));
      at += size;
    }
  }
  const ecBlocks = dataBlocks.map((block) => rsRemainder(block, spec.ec));
  const out: number[] = [];
  const longest = Math.max(...dataBlocks.map((b) => b.length));
  for (let i = 0; i < longest; i++) for (const b of dataBlocks) if (i < b.length) out.push(b[i]);
  for (let i = 0; i < spec.ec; i++) for (const b of ecBlocks) out.push(b[i]);
  return out;
}

function bch(value: number, poly: number, valueBits: number): number {
  const polyLen = Math.floor(Math.log2(poly));
  let v = value << polyLen;
  for (let i = valueBits + polyLen - 1; i >= polyLen; i--) {
    if ((v >>> i) & 1) v ^= poly << (i - polyLen);
  }
  return (value << polyLen) | v;
}

/** 15-bit format string for level M and `mask`. */
export function formatBits(mask: number): number {
  return bch(mask, 0x537, 5) ^ 0x5412;
}

function versionBits(version: number): number {
  return bch(version, 0x1f25, 6);
}

const MASKS: ((r: number, c: number) => boolean)[] = [
  (r, c) => (r + c) % 2 === 0,
  (r) => r % 2 === 0,
  (_r, c) => c % 3 === 0,
  (r, c) => (r + c) % 3 === 0,
  (r, c) => (Math.floor(r / 2) + Math.floor(c / 3)) % 2 === 0,
  (r, c) => ((r * c) % 2) + ((r * c) % 3) === 0,
  (r, c) => (((r * c) % 2) + ((r * c) % 3)) % 2 === 0,
  (r, c) => (((r + c) % 2) + ((r * c) % 3)) % 2 === 0,
];

function build(version: number, codewords: number[], mask: number): Matrix {
  const size = 17 + 4 * version;
  const mod: Matrix = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const fn: Matrix = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const set = (r: number, c: number, dark: boolean) => {
    mod[r][c] = dark;
    fn[r][c] = true;
  };

  const finder = (r0: number, c0: number) => {
    for (let dr = -1; dr <= 7; dr++) {
      for (let dc = -1; dc <= 7; dc++) {
        const r = r0 + dr;
        const c = c0 + dc;
        if (r < 0 || c < 0 || r >= size || c >= size) continue;
        const ring = Math.max(Math.abs(dr - 3), Math.abs(dc - 3));
        set(r, c, ring !== 2 && ring !== 4);
      }
    }
  };
  finder(0, 0);
  finder(0, size - 7);
  finder(size - 7, 0);

  for (let i = 8; i < size - 8; i++) {
    set(6, i, i % 2 === 0);
    set(i, 6, i % 2 === 0);
  }

  const pos = alignmentPositions(version);
  for (const r of pos) {
    for (const c of pos) {
      const last = pos[pos.length - 1];
      if ((r === 6 && c === 6) || (r === 6 && c === last) || (r === last && c === 6)) continue;
      for (let dr = -2; dr <= 2; dr++) {
        for (let dc = -2; dc <= 2; dc++) {
          set(r + dr, c + dc, Math.max(Math.abs(dr), Math.abs(dc)) !== 1);
        }
      }
    }
  }

  const format = formatBits(mask);
  const fbit = (i: number) => ((format >>> i) & 1) === 1;
  for (let i = 0; i <= 5; i++) set(i, 8, fbit(i));
  set(7, 8, fbit(6));
  set(8, 8, fbit(7));
  set(8, 7, fbit(8));
  for (let i = 9; i < 15; i++) set(8, 14 - i, fbit(i));
  for (let i = 0; i < 8; i++) set(8, size - 1 - i, fbit(i));
  for (let i = 8; i < 15; i++) set(size - 15 + i, 8, fbit(i));
  set(size - 8, 8, true);

  if (version >= 7) {
    const v = versionBits(version);
    for (let i = 0; i < 18; i++) {
      const dark = ((v >>> i) & 1) === 1;
      const a = Math.floor(i / 3);
      const b = (i % 3) + size - 11;
      set(a, b, dark);
      set(b, a, dark);
    }
  }

  let bit = 0;
  const total = codewords.length * 8;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let vert = 0; vert < size; vert++) {
      for (let j = 0; j < 2; j++) {
        const c = right - j;
        const upward = ((right + 1) & 2) === 0;
        const r = upward ? size - 1 - vert : vert;
        if (fn[r][c]) continue;
        let dark = false;
        if (bit < total) dark = ((codewords[bit >>> 3] >>> (7 - (bit & 7))) & 1) === 1;
        bit++;
        mod[r][c] = dark !== MASKS[mask](r, c);
      }
    }
  }
  return mod;
}

const FINDER_LIKE = [true, false, true, true, true, false, true];

function penalty(m: Matrix): number {
  const n = m.length;
  let score = 0;
  const scan = (get: (i: number, j: number) => boolean) => {
    for (let i = 0; i < n; i++) {
      let run = 1;
      for (let j = 1; j <= n; j++) {
        if (j < n && get(i, j) === get(i, j - 1)) run++;
        else {
          if (run >= 5) score += run - 2;
          run = 1;
        }
      }
      for (let j = 0; j + 7 <= n; j++) {
        if (!FINDER_LIKE.every((v, k) => get(i, j + k) === v)) continue;
        if (j >= 4 && [1, 2, 3, 4].every((k) => !get(i, j - k))) score += 40;
        if (j + 11 <= n && [7, 8, 9, 10].every((k) => !get(i, j + k))) score += 40;
      }
    }
  };
  scan((i, j) => m[i][j]);
  scan((i, j) => m[j][i]);
  for (let r = 0; r < n - 1; r++) {
    for (let c = 0; c < n - 1; c++) {
      const v = m[r][c];
      if (v === m[r][c + 1] && v === m[r + 1][c] && v === m[r + 1][c + 1]) score += 3;
    }
  }
  let dark = 0;
  for (const row of m) for (const v of row) if (v) dark++;
  score += Math.floor(Math.abs((dark * 100) / (n * n) - 50) / 5) * 10;
  return score;
}

/** Longest input, in UTF-8 bytes, that fits. */
export const QR_MAX_BYTES = 2331;

/** Encode `text` (UTF-8) as a QR matrix; `true` is a dark module. */
export function qrMatrix(text: string): boolean[][] {
  const bytes = Array.from(new TextEncoder().encode(text));
  let version = 0;
  for (let v = 1; v <= SPECS.length; v++) {
    const header = 4 + (v >= 10 ? 16 : 8);
    if (header + bytes.length * 8 <= dataCapacity(SPECS[v - 1]) * 8) {
      version = v;
      break;
    }
  }
  if (version === 0) {
    throw new RangeError(`QR text is ${bytes.length} bytes; at most ${QR_MAX_BYTES} fit`);
  }
  const codewords = encodeCodewords(bytes, version);
  let best: Matrix | null = null;
  let bestScore = Infinity;
  for (let mask = 0; mask < 8; mask++) {
    const m = build(version, codewords, mask);
    const s = penalty(m);
    if (s < bestScore) {
      best = m;
      bestScore = s;
    }
  }
  return best as Matrix;
}

/** One SVG path covering every dark module (horizontal runs merged), in module units. */
export function qrSvgPath(matrix: boolean[][], margin = 0): string {
  const parts: string[] = [];
  matrix.forEach((row, r) => {
    let c = 0;
    while (c < row.length) {
      if (!row[c]) {
        c++;
        continue;
      }
      let end = c;
      while (end < row.length && row[end]) end++;
      parts.push(`M${c + margin} ${r + margin}h${end - c}v1h${c - end}z`);
      c = end;
    }
  });
  return parts.join("");
}
