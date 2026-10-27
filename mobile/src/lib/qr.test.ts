import { describe, expect, it } from "vitest";

import { formatBits, QR_MAX_BYTES, qrMatrix, qrSvgPath } from "./qr";

const rows = (m: boolean[][]) => m.map((r) => r.map((v) => (v ? "#" : ".")).join(""));

// Both goldens were checked cell for cell against `qrencode -l M -m 0` (libqrencode).
const GOLDEN_UNICODE = [
  "#######..#.##.#######", "#.....#....#..#.....#", "#.###.#.###...#.###.#", "#.###.#.##.##.#.###.#",
  "#.###.#.#.#.#.#.###.#", "#.....#.##.#..#.....#", "#######.#.#.#.#######", "........##...........",
  "#.#####...##..#####..", "#.#.#.....#.#####...#", "####..#.##..#.#..##.#", "#.......#.####...#.##",
  ".#...###..###.#......", "........#.....#.#.#..", "#######...####.....##", "#.....#.#...##.#...#.",
  "#.###.#.#####.##.####", "#.###.#.#......###...", "#.###.#.#..#####..#..", "#.....#..#.#.#..#....",
  "#######.##.#..##.###.",
];

const GOLDEN_URL = [
  "#######..#....#.#..###.#..#######", "#.....#..##.#..##...#...#.#.....#", "#.###.#.###...#.#.#.###...#.###.#",
  "#.###.#.#...#....##...#.#.#.###.#", "#.###.#.###...###..###.#..#.###.#", "#.....#.#..#.#......###.#.#.....#",
  "#######.#.#.#.#.#.#.#.#.#.#######", "........###.#...#.#..###.........", "#.#####..#.#.##..#.##...#.#####..",
  "##..##..##.###..#####..#..##.####", "#####.##.....###....###..##.#.#..", "..####.#..##.#.#.....#...#..####.",
  "#.....##.#..#.#.##....#.##..##.#.", "##..##.#...###.#.####..#####.#.##", "#.######..#.#..##.#.#.#..##..#.#.",
  "##..##...#..####..####...##..##..", "##.#..#..##.##.#.###..#..#.##...#", "#.##....####....#.###..#..##.##.#",
  "..#######.#.#.##..#.##...#.##.##.", "##.#...###.###.##..####.#..####.#", "....#.#......###.#.##.#.##.###..#",
  "##..##.#.#...#.####.##.#.##...#.#", "#.##.###.#...#...#...#.....#.#.#.", "#.#..#.###...##.#...###..##..###.",
  "#..##.#.##.#.##.##....#.#####...#", "........##.##...#.####..#...#.###", "#######.....#######.#####.#.#.##.",
  "#.....#.#...#.###.#######...####.", "#.###.#.#..#####.#.##.#.######..#", "#.###.#.#..#..##..###...##..##.##",
  "#.###.#.#.#######.#..#.#..##.#...", "#.....#..#.###.....#####....###..", "#######.#.###..###.#...##.##...#.",
];

describe("qrMatrix", () => {
  it("matches reference encoder output for fixed inputs", () => {
    expect(rows(qrMatrix("é✓ unicode"))).toEqual(GOLDEN_UNICODE);
    expect(rows(qrMatrix("https://claude.ai/oauth/authorize?code=true&client_id=abc"))).toEqual(GOLDEN_URL);
  });

  it("grows the version with the input and stays square", () => {
    const sizes = [1, 14, 15, 26, 27, 62, 63, 122, 123, 152, 153, 180, 181, QR_MAX_BYTES].map(
      (n) => qrMatrix("a".repeat(n)).length,
    );
    expect(sizes[0]).toBe(21);
    expect(sizes[1]).toBe(21);
    expect(sizes[2]).toBe(25);
    expect(sizes[sizes.length - 1]).toBe(177);
    for (let i = 1; i < sizes.length; i++) expect(sizes[i]).toBeGreaterThanOrEqual(sizes[i - 1]);
    for (const s of sizes) expect((s - 17) % 4).toBe(0);
  });

  it("draws finder, timing and dark-module structure", () => {
    for (const text of ["a", "b".repeat(100), "c".repeat(200)]) {
      const m = qrMatrix(text);
      const n = m.length;
      for (const [r0, c0] of [[0, 0], [0, n - 7], [n - 7, 0]]) {
        for (let r = 0; r < 7; r++) {
          for (let c = 0; c < 7; c++) {
            const ring = Math.max(Math.abs(r - 3), Math.abs(c - 3));
            expect(m[r0 + r][c0 + c]).toBe(ring !== 2);
          }
        }
      }
      for (let i = 8; i < n - 8; i++) {
        expect(m[6][i]).toBe(i % 2 === 0);
        expect(m[i][6]).toBe(i % 2 === 0);
      }
      expect(m[n - 8][8]).toBe(true);
    }
  });

  it("writes a valid BCH format word, twice, and a version word from version 7", () => {
    const m = qrMatrix("d".repeat(100));
    const n = m.length;
    const read = (cells: [number, number][]) =>
      cells.reduce((acc, [r, c], i) => acc | ((m[r][c] ? 1 : 0) << i), 0);
    const first = read([
      [0, 8], [1, 8], [2, 8], [3, 8], [4, 8], [5, 8], [7, 8], [8, 8], [8, 7],
      [8, 5], [8, 4], [8, 3], [8, 2], [8, 1], [8, 0],
    ]);
    const second = read([
      ...[0, 1, 2, 3, 4, 5, 6, 7].map((i): [number, number] => [8, n - 1 - i]),
      ...[8, 9, 10, 11, 12, 13, 14].map((i): [number, number] => [n - 15 + i, 8]),
    ]);
    expect(second).toBe(first);
    expect([0, 1, 2, 3, 4, 5, 6, 7].map(formatBits)).toContain(first);
    // 4 data bytes... any word is a multiple of the generator once the mask is removed.
    let rem = first ^ 0x5412;
    for (let i = 14; i >= 10; i--) if ((rem >>> i) & 1) rem ^= 0x537 << (i - 10);
    expect(rem).toBe(0);

    const big = qrMatrix("e".repeat(130)); // version 8
    const bn = big.length;
    let word = 0;
    for (let i = 0; i < 18; i++) {
      if (big[Math.floor(i / 3)][(i % 3) + bn - 11]) word |= 1 << i;
    }
    expect(word >>> 12).toBe(8);
    let vrem = word;
    for (let i = 17; i >= 12; i--) if ((vrem >>> i) & 1) vrem ^= 0x1f25 << (i - 12);
    expect(vrem).toBe(0);
  });

  it("is deterministic and refuses input that does not fit", () => {
    expect(qrMatrix("hello")).toEqual(qrMatrix("hello"));
    expect(() => qrMatrix("x".repeat(QR_MAX_BYTES + 1))).toThrow(RangeError);
    expect(() => qrMatrix("é".repeat(1200))).toThrow(RangeError);
  });
});

const OAUTH_FRAGMENT =
  "oauth/authorize?response_type=code&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback&scope=org%3Acreate_api_key+user%3Aprofile&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&state=";

function oauthLike(length: number): string {
  let text = "https://claude.ai/";
  while (text.length < length) text += OAUTH_FRAGMENT;
  return text.slice(0, length);
}

function fnv1a(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 0x01000193) >>> 0;
  return h;
}

describe("larger versions", () => {
  // Hashes of matrices verified cell for cell against `qrencode -l M` and decoded with zbarimg.
  // Versions 13, 16, 19 and 32 (3, 4, 4 and 6 alignment centres per row), plus the maximum.
  const goldens: [number, number, number][] = [
    [300, 69, 0x16f2bb85],
    [450, 81, 0x12fa6d44],
    [600, 93, 0xa9ef1167],
    [1500, 145, 0x27312280],
    [2331, 177, 0x514d7185],
  ];
  it.each(goldens)("encodes a %i byte sign-in link identically to the reference", (len, size, hash) => {
    const m = qrMatrix(oauthLike(len));
    expect(m.length).toBe(size);
    expect(fnv1a(rows(m).join("\n"))).toBe(hash);
  });

  it("switches version exactly at the level M data capacity", () => {
    // [data codewords, size]: bytes that fit = codewords - 2 (versions 1-9) or - 3 (10+).
    for (const [codewords, size, nextSize] of [
      [254, 61, 65],
      [365, 73, 77],
      [1000, 117, 121],
      [2334, 177, 177],
    ]) {
      const fit = codewords - 3;
      expect(qrMatrix("a".repeat(fit)).length).toBe(size);
      if (codewords !== 2334) expect(qrMatrix("a".repeat(fit + 1)).length).toBe(nextSize);
    }
  });

  it("draws alignment patterns at every centre of a version with six per row", () => {
    const m = qrMatrix(oauthLike(1500)); // version 32
    const centres = [6, 34, 60, 86, 112, 138];
    for (const r of centres) {
      for (const c of centres) {
        const finderCorner = (r === 6 && (c === 6 || c === 138)) || (r === 138 && c === 6);
        if (finderCorner) continue;
        for (let dr = -2; dr <= 2; dr++) {
          for (let dc = -2; dc <= 2; dc++) {
            expect(m[r + dr][c + dc]).toBe(Math.max(Math.abs(dr), Math.abs(dc)) !== 1);
          }
        }
      }
    }
  });
});

describe("qrSvgPath", () => {
  it("covers exactly the dark modules", () => {
    const m = qrMatrix("https://example.com/device?code=ABCD-1234");
    const margin = 4;
    const painted = new Set<string>();
    for (const [, x, y, w] of qrSvgPath(m, margin).matchAll(/M(\d+) (\d+)h(\d+)v1h-\d+z/g)) {
      for (let i = 0; i < Number(w); i++) {
        const key = `${Number(x) + i},${y}`;
        expect(painted.has(key)).toBe(false);
        painted.add(key);
      }
    }
    const dark = new Set<string>();
    m.forEach((row, r) => row.forEach((v, c) => v && dark.add(`${c + margin},${r + margin}`)));
    expect(painted).toEqual(dark);
  });

  it("is empty for an empty matrix", () => {
    expect(qrSvgPath([])).toBe("");
  });
});
