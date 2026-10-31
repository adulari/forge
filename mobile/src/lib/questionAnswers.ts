import type { FormAnswer, FormQuestion } from "./ws";

export interface AnswerDraft {
  selected: string[];
  other: string;
  note: string;
}

/** Wire text for a structured `ask_user` answer: `{"answers":[{selected,other,note}]}`. */
export function formAnswerText(drafts: AnswerDraft[]): string {
  const answers: FormAnswer[] = drafts.map((d) => ({
    selected: d.selected,
    other: d.other.trim().length > 0 ? d.other.trim() : null,
    note: d.note.trim().length > 0 ? d.note.trim() : null,
  }));
  return JSON.stringify({ answers });
}

/** Local one-word readout of an answer; a secret never shows its text. */
export function answerSummary(q: FormQuestion, d: AnswerDraft): string {
  if (q.secret && d.other.trim().length > 0) return "hidden";
  return d.selected.length > 0 ? d.selected.join(", ") : d.other.trim();
}
