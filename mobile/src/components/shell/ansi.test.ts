import { describe, expect, it } from "vitest";

import { AnsiScrollback, lineText } from "./ansi";

const render = (buffer: AnsiScrollback) => buffer.lines().map(lineText);

describe("AnsiScrollback", () => {
  it("splits on newlines and keeps an in-progress line", () => {
    const buffer = new AnsiScrollback();
    buffer.write("one\ntwo");
    expect(render(buffer)).toEqual(["one", "two"]);
  });

  it("applies SGR colours as spans without printing the escape", () => {
    const buffer = new AnsiScrollback();
    buffer.write("\x1b[32mok\x1b[0m done");
    const [line] = buffer.lines();
    expect(lineText(line)).toBe("ok done");
    expect(line.spans[0].style.fg).toEqual({ kind: "basic", index: 2 });
    expect(line.spans[1].style.fg).toBeNull();
  });

  it("swallows unsupported escapes rather than printing them", () => {
    const buffer = new AnsiScrollback();
    // OSC title, alternate-screen mode switch, cursor home, charset designator.
    buffer.write("\x1b]0;title\x07\x1b[?1049h\x1b[2;5H\x1b(Bplain");
    expect(render(buffer)).toEqual(["plain"]);
  });

  it("reassembles an escape split across two chunks", () => {
    const buffer = new AnsiScrollback();
    buffer.write("a\x1b[3");
    buffer.write("1mred");
    const [line] = buffer.lines();
    expect(lineText(line)).toBe("ared");
    expect(line.spans[1].style.fg).toEqual({ kind: "basic", index: 1 });
  });

  it("overwrites from column 0 on carriage return", () => {
    const buffer = new AnsiScrollback();
    buffer.write("50%\r100%");
    expect(render(buffer)).toEqual(["100%"]);
  });

  it("erases to end of line", () => {
    const buffer = new AnsiScrollback();
    buffer.write("abcdef\r\x1b[Kxy");
    expect(render(buffer)).toEqual(["xy"]);
  });

  it("expands tabs to the next 8-column stop", () => {
    const buffer = new AnsiScrollback();
    buffer.write("ab\tc");
    expect(render(buffer)).toEqual(["ab      c"]);
  });

  it("resolves 256-colour and truecolour to rgb", () => {
    const buffer = new AnsiScrollback();
    buffer.write("\x1b[38;5;196ma\x1b[38;2;10;20;30mb");
    const [line] = buffer.lines();
    expect(line.spans[0].style.fg).toEqual({ kind: "rgb", r: 255, g: 0, b: 0 });
    expect(line.spans[1].style.fg).toEqual({ kind: "rgb", r: 10, g: 20, b: 30 });
  });

  it("bounds the scrollback", () => {
    const buffer = new AnsiScrollback(3);
    for (let i = 0; i < 10; i += 1) buffer.write(`line ${i}\n`);
    // 3 retained completed lines + the empty in-progress one.
    expect(render(buffer)).toEqual(["line 7", "line 8", "line 9", ""]);
  });

  it("clears on erase-display", () => {
    const buffer = new AnsiScrollback();
    buffer.write("old\nstuff\n\x1b[2Jfresh");
    expect(render(buffer)).toEqual(["fresh"]);
  });
});
