import { describe, expect, it } from "vitest";

import { answerSummary, formAnswerText } from "./questionAnswers";
import type { FormQuestion } from "./ws";

const q = (secret?: boolean): FormQuestion => ({ header: "", text: "Token?", options: [], multi: false, allow_other: true, allow_note: false, secret });

describe("formAnswerText", () => {
  it("keeps the structured wire shape, trimming text and nulling empties", () => {
    const text = formAnswerText([{ selected: [], other: "  hunter2 ", note: "" }, { selected: ["Yes"], other: "", note: "n" }]);
    expect(JSON.parse(text)).toEqual({
      answers: [
        { selected: [], other: "hunter2", note: null },
        { selected: ["Yes"], other: null, note: "n" },
      ],
    });
  });
});

describe("answerSummary", () => {
  it("never shows a secret", () => {
    expect(answerSummary(q(true), { selected: [], other: "hunter2", note: "" })).toBe("hidden");
  });

  it("shows ordinary free text", () => {
    expect(answerSummary(q(false), { selected: [], other: "blue", note: "" })).toBe("blue");
    expect(answerSummary(q(), { selected: ["A", "B"], other: "", note: "" })).toBe("A, B");
  });
});
