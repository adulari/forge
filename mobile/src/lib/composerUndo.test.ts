import { describe, expect, it } from "vitest";

import { EMPTY_EDITS, recordEdit, redoEdit, undoEdit } from "./composerUndo";

describe("composer programmatic undo", () => {
  it("undoes and redoes a voice insert", () => {
    const stack = recordEdit(EMPTY_EDITS, "fix the", "fix the bug");
    const undone = undoEdit(stack, "fix the bug");
    expect(undone?.text).toBe("fix the");
    const redone = redoEdit(undone!.stack, "fix the");
    expect(redone?.text).toBe("fix the bug");
  });

  it("steps back through several edits in order", () => {
    let stack = recordEdit(EMPTY_EDITS, "", "@a.ts");
    stack = recordEdit(stack, "@a.ts", "@a.ts @b.ts");
    const first = undoEdit(stack, "@a.ts @b.ts");
    expect(first?.text).toBe("@a.ts");
    expect(undoEdit(first!.stack, "@a.ts")?.text).toBe("");
  });

  it("stays out of the way once the draft was typed into", () => {
    const stack = recordEdit(EMPTY_EDITS, "a", "a b");
    expect(undoEdit(stack, "a b c")).toBeNull();
  });

  it("a new edit forgets the redo branch; a no-op edit records nothing", () => {
    const stack = recordEdit(EMPTY_EDITS, "a", "b");
    const undone = undoEdit(stack, "b")!;
    expect(recordEdit(undone.stack, "a", "c").redo).toEqual([]);
    expect(recordEdit(EMPTY_EDITS, "same", "same")).toBe(EMPTY_EDITS);
  });
});
