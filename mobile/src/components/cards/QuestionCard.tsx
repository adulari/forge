// DESIGN_SYSTEM.md §6 QuestionCard: the `ask_user` form. One or more questions answered in
// place — a stepper strip when several were asked, radio rows (single) or checkbox rows
// (multi-select), an "Other…" free-text row and an optional note per question, then one
// send for the whole form.
//
// ARCHITECTURE.md §3 prompt_seq discipline: the form answers ONCE via
// `answer{text: JSON.stringify({answers}), seq}`; rows/inputs disable after the send until a new
// `prompt_seq` arrives; never retry. A pre-v11 host (no `question_form`) gets the original
// single-question card, which answers the 1-based option number as a STRING.
import { Check, ChevronLeft, ChevronRight, Send } from "lucide-react-native";
import { useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { type FormAnswer, type FormQuestion, type QuestionOption, type RemoteInput } from "../../lib/ws";
import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface QuestionCardProps {
  question: string;
  options: QuestionOption[];
  allowOther: boolean;
  /** The whole form when the host sends one (v11); null/undefined = single-question host. */
  form?: FormQuestion[] | null;
  promptSeq: number;
  send: (input: RemoteInput) => boolean;
  onQueueAnswer?: (input: Extract<RemoteInput, { kind: "allow" | "answer" }>) => void;
}

interface Draft {
  selected: string[];
  other: string;
  note: string;
}

const emptyDraft = (): Draft => ({ selected: [], other: "", note: "" });
const answered = (d: Draft): boolean => d.selected.length > 0 || d.other.trim().length > 0;

export function QuestionCard(props: QuestionCardProps) {
  const { form, promptSeq, send, onQueueAnswer } = props;
  const toast = useToast();
  const [lockedSeq, setLockedSeq] = useState<number | null>(null);
  const [queued, setQueued] = useState(false);

  useEffect(() => {
    setLockedSeq(null);
    setQueued(false);
  }, [promptSeq]);

  const locked = lockedSeq === promptSeq;

  const deliver = (text: string) => {
    if (locked || text.trim().length === 0) return;
    setLockedSeq(promptSeq);
    haptics.select();
    if (!send({ kind: "answer", text, seq: promptSeq })) {
      if (onQueueAnswer) {
        onQueueAnswer({ kind: "answer", text, seq: promptSeq });
        setQueued(true);
      } else {
        setLockedSeq(null);
        toast.show("not sent — reconnect and try again", { tone: "danger" });
      }
      haptics.mergeConflict();
    }
  };

  // A form host: every question is a FormQuestion, even a single legacy-shaped one.
  const questions: FormQuestion[] = useMemo(
    () =>
      form && form.length > 0
        ? form
        : [
            {
              header: "",
              text: props.question,
              options: props.options,
              multi: false,
              allow_other: props.allowOther || props.options.length === 0,
              allow_note: false,
            },
          ],
    [form, props.question, props.options, props.allowOther],
  );
  const structured = form != null && form.length > 0;

  return (
    <FormBody
      key={promptSeq}
      questions={questions}
      locked={locked}
      queued={queued}
      onSubmit={(drafts) => {
        if (structured) {
          const answers: FormAnswer[] = drafts.map((d) => ({
            selected: d.selected,
            other: d.other.trim().length > 0 ? d.other.trim() : null,
            note: d.note.trim().length > 0 ? d.note.trim() : null,
          }));
          deliver(JSON.stringify({ answers }));
        } else {
          // Legacy host: one question, answer by option number or free text.
          const d = drafts[0];
          const idx = questions[0].options.findIndex((o) => o.label === d.selected[0]);
          deliver(idx >= 0 ? String(idx + 1) : d.other);
        }
      }}
    />
  );
}

function FormBody({
  questions,
  locked,
  queued,
  onSubmit,
}: {
  questions: FormQuestion[];
  locked: boolean;
  queued: boolean;
  onSubmit: (drafts: Draft[]) => void;
}) {
  const tokens = useTokens();
  const [drafts, setDrafts] = useState<Draft[]>(() => questions.map(emptyDraft));
  const [current, setCurrent] = useState(0);
  const [showNote, setShowNote] = useState(false);

  const q = questions[current];
  const d = drafts[current] ?? emptyDraft();
  const multi = questions.length > 1;
  const last = current === questions.length - 1;
  const firstUnanswered = drafts.findIndex((x) => !answered(x));
  const complete = firstUnanswered < 0;

  const update = (patch: Partial<Draft>) =>
    setDrafts((prev) => prev.map((x, i) => (i === current ? { ...x, ...patch } : x)));

  const toggle = (label: string) => {
    if (locked) return;
    haptics.select();
    if (q.multi) {
      update({
        selected: d.selected.includes(label) ? d.selected.filter((l) => l !== label) : [...d.selected, label],
      });
    } else {
      update({ selected: [label], other: "" });
    }
  };

  const next = () => {
    if (!last) {
      setCurrent(current + 1);
      setShowNote(false);
      return;
    }
    if (!complete) {
      setCurrent(firstUnanswered);
      setShowNote(false);
      return;
    }
    onSubmit(drafts);
  };

  const canAdvance = answered(d);
  const showOther = q.allow_other || q.options.length === 0;

  return (
    <Card variant="feature" style={[styles.container, { borderColor: hexToRgba(tokens.accent, 0.45) }]}>
      {multi ? (
        <View style={styles.stepper}>
          {questions.map((qi, i) => {
            const isCurrent = i === current;
            const done = answered(drafts[i] ?? emptyDraft());
            return (
              <Pressable
                key={i}
                onPress={() => {
                  setCurrent(i);
                  setShowNote(false);
                }}
                accessibilityRole="tab"
                accessibilityState={{ selected: isCurrent }}
                style={[
                  styles.step,
                  {
                    backgroundColor: isCurrent ? tokens.accent : done ? tokens.successBg : tokens.bg3,
                    borderRadius: radii.radius8,
                  },
                ]}
              >
                {done && !isCurrent ? <Check size={12} strokeWidth={2.5} color={tokens.success} /> : null}
                <Text
                  style={[
                    typeScale.meta,
                    { color: isCurrent ? tokens.onAccent : done ? tokens.success : tokens.ink2 },
                  ]}
                  numberOfLines={1}
                >
                  {qi.header || `${i + 1}`}
                </Text>
              </Pressable>
            );
          })}
        </View>
      ) : null}

      <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{q.text}</Text>
      {q.multi ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>choose all that apply</Text> : null}
      {queued ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>will send on reconnect</Text> : null}

      {q.options.map((opt) => (
        <OptionRow
          key={opt.label}
          option={opt}
          multi={q.multi}
          selected={d.selected.includes(opt.label)}
          disabled={locked}
          onPress={() => toggle(opt.label)}
        />
      ))}

      {showOther ? (
        <Input
          value={d.other}
          onChangeText={(t) => update(q.multi ? { other: t } : { other: t, selected: [] })}
          placeholder={q.options.length === 0 ? "type your answer…" : "other — type your own answer…"}
          editable={!locked}
          returnKeyType={last ? "send" : "next"}
          onSubmitEditing={() => (canAdvance ? next() : undefined)}
          containerStyle={styles.field}
          accessibilityLabel="free-text answer"
        />
      ) : null}

      {q.allow_note ? (
        showNote || d.note.length > 0 ? (
          <Input
            value={d.note}
            onChangeText={(t) => update({ note: t })}
            placeholder="add a note for the model (optional)"
            editable={!locked}
            containerStyle={styles.field}
            accessibilityLabel="note"
          />
        ) : (
          <Pressable onPress={() => setShowNote(true)} disabled={locked} accessibilityRole="button" hitSlop={8}>
            <Text style={[typeScale.sub, { color: tokens.accent }]}>+ add a note</Text>
          </Pressable>
        )
      ) : null}

      <View style={styles.footer}>
        {multi ? (
          <IconButton
            icon={<ChevronLeft size={20} strokeWidth={1.75} color={current === 0 ? tokens.ink3 : tokens.ink} />}
            onPress={() => {
              setCurrent(Math.max(0, current - 1));
              setShowNote(false);
            }}
            disabled={locked || current === 0}
            accessibilityLabel="previous question"
          />
        ) : (
          <View />
        )}
        <Text style={[typeScale.meta, { color: tokens.ink3 }]}>
          {multi ? `${drafts.filter(answered).length}/${questions.length} answered` : ""}
        </Text>
        {last ? (
          <Button
            label={multi ? "send answers" : "send"}
            variant="primary"
            icon={<Send size={16} strokeWidth={1.75} color={tokens.onAccent} />}
            onPress={next}
            disabled={locked || !canAdvance}
            accessibilityLabel={multi ? "send all answers" : "send answer"}
          />
        ) : (
          <Button
            label="next"
            variant="secondary"
            icon={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink} />}
            onPress={next}
            disabled={locked || !canAdvance}
            accessibilityLabel="next question"
          />
        )}
      </View>
    </Card>
  );
}

function OptionRow({
  option,
  multi,
  selected,
  disabled,
  onPress,
}: {
  option: QuestionOption;
  multi: boolean;
  selected: boolean;
  disabled: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  const strike = useStrike();

  return (
    <Animated.View style={strike.style}>
      <Pressable
        onPress={disabled ? undefined : onPress}
        onPressIn={disabled ? undefined : strike.onPressIn}
        onPressOut={disabled ? undefined : strike.onPressOut}
        disabled={disabled}
        accessibilityRole={multi ? "checkbox" : "radio"}
        accessibilityLabel={option.description ? `${option.label} — ${option.description}` : option.label}
        accessibilityState={{ disabled, checked: selected, selected }}
        style={[
          styles.option,
          {
            backgroundColor: selected ? hexToRgba(tokens.accent, 0.12) : tokens.bg3,
            borderColor: selected ? tokens.accent : "transparent",
            borderRadius: radii.radius8,
            opacity: disabled ? 0.4 : 1,
          },
        ]}
      >
        <View
          style={[
            styles.mark,
            {
              borderRadius: multi ? radii.radius4 : 11,
              borderColor: selected ? tokens.accent : tokens.borderStrong,
              backgroundColor: selected ? tokens.accent : "transparent",
            },
          ]}
        >
          {selected ? <Check size={14} strokeWidth={2.5} color={tokens.onAccent} /> : null}
        </View>
        <View style={styles.optionText}>
          <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{option.label}</Text>
          {option.description ? (
            <Text style={[typeScale.sub, { color: tokens.ink2 }, styles.optionDetail]}>{option.description}</Text>
          ) : null}
        </View>
      </Pressable>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  container: { gap: space.space8 },
  stepper: { flexDirection: "row", flexWrap: "wrap", gap: space.space4, marginBottom: space.space4 },
  step: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    maxWidth: 140,
  },
  option: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    paddingHorizontal: space.space12,
    paddingVertical: space.space12,
    minHeight: 44,
    borderWidth: 1,
  },
  mark: { width: 22, height: 22, borderWidth: 1.5, alignItems: "center", justifyContent: "center" },
  optionText: { flex: 1 },
  optionDetail: { marginTop: space.space2 },
  field: { marginTop: space.space4 },
  footer: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", marginTop: space.space4 },
});
