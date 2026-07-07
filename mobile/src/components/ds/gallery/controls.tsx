// T1.1 controls gallery — every ds control in every reachable state, at a glance.
// T1.3 owns the route (`src/app/gallery.tsx`) that mounts this component.
// P (pressed/Strike) and F (focused) are live interaction states — tap/tab into a
// control to see them; this screen otherwise shows every state that can be forced.
import React, { useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { Paperclip } from "lucide-react-native";

import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { type } from "../../../theme/typography";
import { Button } from "../Button";
import { Checkbox } from "../Checkbox";
import { Chip } from "../Chip";
import { IconButton } from "../IconButton";
import { Input } from "../Input";
import { SearchField } from "../SearchField";
import { Segmented } from "../Segmented";
import { Switch } from "../Switch";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  const tokens = useTokens();
  return (
    <View style={styles.section}>
      <Text style={[type.section, { color: tokens.ink3, marginBottom: space.space12 }]}>{title}</Text>
      <View style={styles.rows}>{children}</View>
    </View>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  const tokens = useTokens();
  return (
    <View style={styles.row}>
      <Text style={[type.meta, { color: tokens.ink3, width: 96 }]}>{label}</Text>
      <View style={styles.rowContent}>{children}</View>
    </View>
  );
}

export default function ControlsGallery() {
  const tokens = useTokens();
  const [searchValue, setSearchValue] = useState("");
  const [inputValue, setInputValue] = useState("");
  const [inputWithText, setInputWithText] = useState("main.rs");
  const [switchOn, setSwitchOn] = useState(true);
  const [switchOff, setSwitchOff] = useState(false);
  const [checked, setChecked] = useState(true);
  const [unchecked, setUnchecked] = useState(false);
  const [selectedChip, setSelectedChip] = useState(true);
  const [segmentValue, setSegmentValue] = useState<"chat" | "tasks" | "agents" | "review">("chat");

  return (
    <ScrollView
      style={{ backgroundColor: tokens.bg1 }}
      contentContainerStyle={styles.content}
      accessibilityLabel="Controls gallery"
    >
      <Section title="Button — primary/secondary/ghost/danger/allow">
        <Row label="primary">
          <Button label="Send prompt" variant="primary" onPress={() => {}} />
        </Row>
        <Row label="secondary">
          <Button label="Cancel" variant="secondary" onPress={() => {}} />
        </Row>
        <Row label="ghost">
          <Button label="Revise" variant="ghost" onPress={() => {}} />
        </Row>
        <Row label="danger">
          <Button label="Deny" variant="danger" onPress={() => {}} />
        </Row>
        <Row label="allow">
          <Button label="Allow" variant="allow" onPress={() => {}} />
        </Row>
        <Row label="loading (L)">
          <Button label="Approve" variant="primary" loading onPress={() => {}} />
        </Row>
        <Row label="disabled (X)">
          <Button label="Approve" variant="primary" disabled onPress={() => {}} />
        </Row>
        <Row label="with icon">
          <Button label="Attach" variant="secondary" icon={<Paperclip size={16} strokeWidth={1.75} color={tokens.ink} />} onPress={() => {}} />
        </Row>
      </Section>

      <Section title="IconButton">
        <Row label="default">
          <IconButton
            icon={<Paperclip size={20} strokeWidth={1.75} color={tokens.ink} />}
            accessibilityLabel="Attach"
            onPress={() => {}}
          />
        </Row>
        <Row label="badge">
          <IconButton
            icon={<Paperclip size={20} strokeWidth={1.75} color={tokens.ink} />}
            accessibilityLabel="Inbox"
            badge
            onPress={() => {}}
          />
        </Row>
        <Row label="disabled (X)">
          <IconButton
            icon={<Paperclip size={20} strokeWidth={1.75} color={tokens.ink} />}
            accessibilityLabel="Attach"
            disabled
            onPress={() => {}}
          />
        </Row>
      </Section>

      <Section title="Input">
        <Row label="default (D)">
          <Input label="Session title" placeholder="New session" value={inputValue} onChangeText={setInputValue} />
        </Row>
        <Row label="with value (clear)">
          <Input label="Path" mono value={inputWithText} onChangeText={setInputWithText} />
        </Row>
        <Row label="error (E)">
          <Input label="Connect URL" mono value="forge://bad-token" onChangeText={() => {}} error="invalid token — re-scan the code" />
        </Row>
        <Row label="disabled (X)">
          <Input label="Server" value="forge.local:8420" onChangeText={() => {}} disabled />
        </Row>
      </Section>

      <Section title="SearchField">
        <Row label="default">
          <SearchField value={searchValue} onChangeText={setSearchValue} onDebouncedChange={() => {}} />
        </Row>
      </Section>

      <Section title="Chip">
        <Row label="unselected / selected">
          <View style={styles.inline}>
            <Chip label="/plan" selected={selectedChip} onPress={() => setSelectedChip((v) => !v)} />
            <Chip label="/compact" onPress={() => {}} />
            <Chip label="/models" onPress={() => {}} />
            <Chip label="disabled" disabled onPress={() => {}} />
          </View>
        </Row>
      </Section>

      <Section title="Segmented">
        <Row label="4 sections">
          <Segmented
            options={[
              { value: "chat", label: "Chat" },
              { value: "tasks", label: "Tasks" },
              { value: "agents", label: "Agents" },
              { value: "review", label: "Review" },
            ]}
            value={segmentValue}
            onChange={setSegmentValue}
          />
        </Row>
      </Section>

      <Section title="Switch">
        <Row label="on">
          <Switch value={switchOn} onValueChange={setSwitchOn} accessibilityLabel="Notifications" />
        </Row>
        <Row label="off">
          <Switch value={switchOff} onValueChange={setSwitchOff} accessibilityLabel="App lock" />
        </Row>
        <Row label="disabled (X)">
          <Switch value onValueChange={() => {}} disabled accessibilityLabel="Locked setting" />
        </Row>
      </Section>

      <Section title="Checkbox">
        <Row label="checked">
          <Checkbox value={checked} onValueChange={setChecked} accessibilityLabel="Use worktree" />
        </Row>
        <Row label="unchecked">
          <Checkbox value={unchecked} onValueChange={setUnchecked} accessibilityLabel="Use worktree" />
        </Row>
        <Row label="disabled (X)">
          <Checkbox value onValueChange={() => {}} disabled accessibilityLabel="Locked setting" />
        </Row>
      </Section>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    padding: space.space16,
    gap: space.space24,
  },
  section: {
    gap: space.space12,
  },
  rows: {
    gap: space.space12,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
  },
  rowContent: {
    flex: 1,
  },
  inline: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: space.space8,
  },
});
