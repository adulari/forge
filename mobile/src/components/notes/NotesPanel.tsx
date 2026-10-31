// The Notes surface: a per-project to-do checklist and a markdown scratchpad. One body shared by
// the desktop/web dock (NotesDock) and the phone page (app/notes.tsx). Every to-do can start a
// thread, which creates a session in the project seeded with the item and remembers the link.
import { router } from "expo-router";
import { ArrowUpRight, NotebookPen, Play, Trash2 } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";

import { scratchpadConflict, type NoteTodo } from "../../lib/apiNotes";
import { useCreateSession } from "../../lib/queries";
import { useNotesAvailable, useNotesMutations, useProjectNotes } from "../../lib/queriesNotes";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale, webInputTextStyle } from "../../theme/typography";
import { Banner } from "../ds/Banner";
import { Button } from "../ds/Button";
import { Checkbox } from "../ds/Checkbox";
import { Chip } from "../ds/Chip";
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { Segmented } from "../ds/Segmented";
import { useToast } from "../ds/ToastHost";
import { Markdown } from "../chat/Markdown";
import {
  EMPTY_PAD,
  padCaption,
  splitTodos,
  threadSeed,
  type NoteProjectChoice,
  type PadState,
} from "./notesModel";
import { PadController } from "./padController";

type Tab = "todos" | "scratchpad";

export interface NotesPanelProps {
  project: string | null;
  projects: readonly NoteProjectChoice[];
  onProject: (path: string) => void;
  /** Side padding. The dock keeps it; a page that already has its own gutter turns it off. */
  inset?: boolean;
}

function TodoRow({
  todo,
  onToggle,
  onEdit,
  onDelete,
  onThread,
  starting,
}: {
  todo: NoteTodo;
  onToggle: (done: boolean) => void;
  onEdit: (text: string) => void;
  onDelete: () => void;
  onThread: () => void;
  starting: boolean;
}) {
  const tokens = useTokens();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(todo.text);
  useEffect(() => {
    if (!editing) setDraft(todo.text);
  }, [todo.text, editing]);

  const commit = () => {
    setEditing(false);
    const next = draft.trim();
    if (next && next !== todo.text) onEdit(next);
    else setDraft(todo.text);
  };

  return (
    <View style={[styles.todo, { borderColor: tokens.border }]}>
      <Checkbox
        value={todo.done}
        onValueChange={onToggle}
        accessibilityLabel={todo.done ? `Mark "${todo.text}" as not done` : `Mark "${todo.text}" as done`}
      />
      <View style={styles.todoBody}>
        {editing ? (
          <TextInput
            autoFocus
            multiline
            value={draft}
            onChangeText={setDraft}
            onBlur={commit}
            onSubmitEditing={commit}
            onKeyPress={(event) => {
              const key = (event.nativeEvent as { key?: string }).key;
              if (key === "Escape") {
                setDraft(todo.text);
                setEditing(false);
              }
            }}
            blurOnSubmit
            accessibilityLabel="Edit to-do"
            style={[typeScale.body, webInputTextStyle, styles.todoInput, { color: tokens.ink, borderColor: tokens.accent }]}
          />
        ) : (
          <Pressable
            onPress={() => setEditing(true)}
            accessibilityRole="button"
            accessibilityLabel={`Edit to-do: ${todo.text}`}
          >
            <Text
              style={[
                typeScale.body,
                { color: todo.done ? tokens.ink3 : tokens.ink },
                todo.done && styles.struck,
              ]}
            >
              {todo.text}
            </Text>
          </Pressable>
        )}
        {todo.session_id ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            thread {todo.session_id.slice(0, 8)}
          </Text>
        ) : null}
      </View>
      {todo.session_id ? (
        <IconButton
          icon={<ArrowUpRight size={16} color={tokens.accent} />}
          accessibilityLabel={`Open the thread for "${todo.text}"`}
          onPress={() => router.push(`/session/${todo.session_id}`)}
        />
      ) : (
        <IconButton
          icon={<Play size={16} color={tokens.ink2} />}
          accessibilityLabel={`Start a thread from "${todo.text}"`}
          onPress={onThread}
          disabled={starting || todo.done}
        />
      )}
      <IconButton
        icon={<Trash2 size={16} color={tokens.ink3} />}
        accessibilityLabel={`Delete "${todo.text}"`}
        onPress={onDelete}
      />
    </View>
  );
}

function Todos({ project }: { project: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const notes = useProjectNotes(project);
  const { add, update, remove, link, clearDone } = useNotesMutations(project);
  const createSession = useCreateSession();
  const [text, setText] = useState("");
  const [starting, setStarting] = useState<string | null>(null);
  const { open, done } = useMemo(() => splitTodos(notes.data?.todos ?? []), [notes.data]);

  const submit = () => {
    const value = text.trim();
    if (!value || add.isPending) return;
    setText("");
    add.mutate(value, {
      onError: (error) => {
        setText(value);
        toast.show(error.message, { tone: "danger" });
      },
    });
  };

  const startThread = async (todo: NoteTodo) => {
    if (starting) return;
    setStarting(todo.id);
    try {
      const seed = threadSeed(todo);
      const session = await createSession.mutateAsync({ cwd: project, title: seed.title, prompt: seed.prompt });
      link.mutate({ id: todo.id, sessionId: session.id });
      router.push(`/session/${session.id}`);
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not start the thread", { tone: "danger" });
    } finally {
      setStarting(null);
    }
  };

  const row = (todo: NoteTodo) => (
    <TodoRow
      key={todo.id}
      todo={todo}
      starting={starting === todo.id}
      onToggle={(value) => update.mutate({ id: todo.id, done: value })}
      onEdit={(value) => update.mutate({ id: todo.id, text: value })}
      onDelete={() => remove.mutate(todo.id)}
      onThread={() => void startThread(todo)}
    />
  );

  return (
    <View style={styles.section}>
      <Input
        value={text}
        onChangeText={setText}
        onSubmitEditing={submit}
        placeholder="Add a to-do"
        returnKeyType="done"
        accessibilityLabel="New to-do"
        clearable={false}
        trailing={<Button label="Add" variant="ghost" onPress={submit} disabled={!text.trim()} />}
      />
      {notes.isPending ? <ActivityIndicator color={tokens.accent} style={styles.loading} /> : null}
      {notes.error ? <Text style={[typeScale.sub, { color: tokens.danger }]}>{notes.error.message}</Text> : null}
      {notes.data && open.length === 0 && done.length === 0 ? (
        <EmptyState icon={NotebookPen} message="Nothing on the list. Add what you want to get to; any item can start a thread." />
      ) : null}
      {open.map(row)}
      {done.length > 0 ? (
        <>
          <View style={styles.doneHead}>
            <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Done ({done.length})</Text>
            <Chip label="Clear done" size="compact" onPress={() => clearDone.mutate()} />
          </View>
          {done.map(row)}
        </>
      ) : null}
    </View>
  );
}

function Scratchpad({ project }: { project: string }) {
  const tokens = useTokens();
  const notes = useProjectNotes(project);
  const { save } = useNotesMutations(project);
  const [pad, setPad] = useState<PadState>(EMPTY_PAD);
  const [preview, setPreview] = useState(false);
  // The controller outlives renders; it is pointed at the current mutation function after each one.
  const [controller] = useState(
    () =>
      new PadController({
        send: () => Promise.reject(new Error("scratchpad is not ready")),
        asConflict: scratchpadConflict,
        onChange: setPad,
      }),
  );
  const { mutateAsync } = save;
  useEffect(() => {
    controller.setSend((body, expectedUpdatedAt) => mutateAsync({ body, expectedUpdatedAt }));
  }, [controller, mutateAsync]);

  useEffect(() => {
    if (notes.data) controller.dispatch({ type: "server", body: notes.data.body, updatedAt: notes.data.updated_at });
  }, [notes.data, controller]);

  // Unmounting (including a switch to another project, which remounts this by key) saves what is left.
  useEffect(() => () => controller.dispose(), [controller]);

  if (notes.isPending) return <ActivityIndicator color={tokens.accent} style={styles.loading} />;
  if (notes.error && !notes.data) return <Text style={[typeScale.sub, { color: tokens.danger }]}>{notes.error.message}</Text>;

  return (
    <View style={styles.section}>
      {pad.conflict ? (
        <Banner
          tone="warn"
          message="This scratchpad was changed on another device."
          actionLabel="Use theirs"
          onAction={() => controller.dispatch({ type: "useTheirs" })}
          secondaryLabel="Keep mine"
          onSecondary={() => controller.dispatch({ type: "keepMine" })}
        />
      ) : null}
      <View style={styles.padHead}>
        <Chip label="Edit" size="compact" selected={!preview} onPress={() => setPreview(false)} />
        <Chip
          label="Preview"
          size="compact"
          selected={preview}
          onPress={() => {
            void controller.flush();
            setPreview(true);
          }}
        />
      </View>
      {preview ? (
        <View style={[styles.preview, { borderColor: tokens.border, backgroundColor: tokens.bg0 }]}>
          {pad.draft.trim() ? (
            <Markdown content={pad.draft} />
          ) : (
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Nothing to preview yet.</Text>
          )}
        </View>
      ) : (
        <TextInput
          multiline
          value={pad.draft}
          onChangeText={(value) => controller.dispatch({ type: "edit", text: value })}
          onBlur={() => void controller.flush()}
          placeholder="Plans, links, reminders. Markdown works."
          placeholderTextColor={tokens.ink3}
          accessibilityLabel="Scratchpad"
          textAlignVertical="top"
          style={[
            typeScale.body,
            webInputTextStyle,
            styles.pad,
            { color: tokens.ink, backgroundColor: tokens.bg0, borderColor: tokens.border, fontFamily: monoFamily.regular },
          ]}
        />
      )}
      <Text style={[typeScale.monoMeta, { color: pad.status === "error" ? tokens.danger : tokens.ink3 }]} accessibilityLiveRegion="polite">
        {padCaption(pad)}
      </Text>
    </View>
  );
}

export function NotesPanel({ project, projects, onProject, inset = true }: NotesPanelProps) {
  const tokens = useTokens();
  const available = useNotesAvailable();
  const notes = useProjectNotes(project);
  const [tab, setTab] = useState<Tab>("todos");
  const openCount = notes.data ? notes.data.todos.filter((todo) => !todo.done).length : 0;

  if (!available) return <EmptyState icon={NotebookPen} message="Notes need a direct connection to the host." />;
  if (!project) return <EmptyState icon={NotebookPen} message="Open a session or pick a project to keep notes for it." />;

  return (
    <ScrollView
      style={{ backgroundColor: tokens.bg0 }}
      contentContainerStyle={[styles.content, !inset && styles.flush]}
      keyboardShouldPersistTaps="handled"
    >
      {projects.length > 1 ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.projects}>
          {projects.map((choice) => (
            <Chip
              key={choice.path}
              label={choice.name}
              size="compact"
              selected={choice.path === project}
              onPress={() => onProject(choice.path)}
            />
          ))}
        </ScrollView>
      ) : null}
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
        {project}
      </Text>
      <Segmented<Tab>
        options={[
          { value: "todos", label: "To-dos", badge: openCount || undefined },
          { value: "scratchpad", label: "Scratchpad" },
        ]}
        value={tab}
        onChange={setTab}
      />
      {tab === "todos" ? <Todos key={project} project={project} /> : <Scratchpad key={project} project={project} />}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.space12, gap: space.space12 },
  flush: { paddingHorizontal: 0 },
  projects: { gap: space.space8 },
  section: { gap: space.space8 },
  loading: { padding: space.space16 },
  todo: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    paddingLeft: space.space8,
    paddingVertical: space.space4,
  },
  todoBody: { flex: 1, gap: space.space2, paddingVertical: space.space4, paddingLeft: space.space4 },
  todoInput: { borderBottomWidth: 1, paddingVertical: space.space2, minHeight: 24 },
  struck: { textDecorationLine: "line-through" },
  doneHead: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", marginTop: space.space8 },
  padHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  pad: {
    minHeight: 280,
    borderWidth: 1,
    borderRadius: radii.radius8,
    padding: space.space12,
  },
  preview: { minHeight: 120, borderWidth: 1, borderRadius: radii.radius8, padding: space.space12 },
});
