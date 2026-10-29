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

describe("OSC 8 hyperlinks", () => {
  const BEL = "\x07";
  const ST = "\x1b\\";

  it("tags the wrapped text with its target and prints neither the sequence nor the target", () => {
    const buffer = new AnsiScrollback();
    buffer.write(`see \x1b]8;;https://forge.dev/docs${BEL}the docs\x1b]8;;${BEL} now`);
    expect(render(buffer)).toEqual(["see the docs now"]);
    const spans = buffer.lines()[0].spans;
    expect(spans.map((span) => [span.text, span.link])).toEqual([
      ["see ", undefined],
      ["the docs", "https://forge.dev/docs"],
      [" now", undefined],
    ]);
  });

  it("accepts the string-terminator form, link params, and a sequence split across frames", () => {
    const buffer = new AnsiScrollback();
    buffer.write("\x1b]8;id=7;https://a.dev/x");
    buffer.write(`${ST}link\x1b]8;;`);
    buffer.write(ST);
    const spans = buffer.lines()[0].spans;
    expect(spans).toHaveLength(1);
    expect(spans[0]).toMatchObject({ text: "link", link: "https://a.dev/x" });
  });

  it("drops a target that is not a URI and forgets the link on clear", () => {
    const buffer = new AnsiScrollback();
    buffer.write(`\x1b]8;;not a uri${BEL}plain`);
    expect(buffer.lines()[0].spans[0].link).toBeUndefined();
    buffer.write(`\x1b]8;;https://a.dev${BEL}`);
    buffer.clear();
    buffer.write("after");
    expect(buffer.lines()[0].spans[0].link).toBeUndefined();
  });
});
