// The shared Find bar: query, hit count, case toggle, previous/next, close. It owns no search of
// its own; it drives lib/find/findController and shows up on whichever host is the active
// target, so every searchable surface looks and behaves the same.
import { ChevronDown, ChevronUp, Search, X } from "lucide-react-native";
import React, { useEffect, useRef } from "react";
import {
  type NativeSyntheticEvent,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  type TextInputKeyPressEventData,
  View,
} from "react-native";

import { type FindResult, type FindTargetId, findController, useFindState } from "../../lib/find/findController";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";

export function findCountLabel(query: string, result: FindResult, searching: boolean): string {
  if (query === "") return "";
  if (result.count === 0) return searching ? "Searching" : "No results";
  const total = `${result.count}${result.capped ? "+" : ""}`;
  return result.index >= 0 ? `${result.index + 1} of ${total}` : total;
}

function BarButton({
  label,
  onPress,
  disabled,
  active,
  children,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  active?: boolean;
  children: React.ReactNode;
}) {
  const tokens = useTokens();
  const [hot, setHot] = React.useState(false);
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      onHoverIn={() => setHot(true)}
      onHoverOut={() => setHot(false)}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled, selected: !!active }}
      hitSlop={4}
      style={[
        styles.button,
        { backgroundColor: active ? tokens.selection : hot && !disabled ? tokens.bg3 : "transparent", opacity: disabled ? 0.4 : 1 },
      ]}
    >
      {children}
    </Pressable>
  );
}

export function FindBar({ targetId }: { targetId: FindTargetId }) {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const find = useFindState();
  const inputRef = useRef<TextInput>(null);
  const mine = find.open && find.targetId === targetId;

  useEffect(() => {
    if (!mine) return;
    const handle = setTimeout(() => inputRef.current?.focus(), 0);
    return () => clearTimeout(handle);
  }, [mine, find.focusNonce]);

  if (!mine) return null;

  const onKeyPress = (event: NativeSyntheticEvent<TextInputKeyPressEventData>) => {
    const { key, shiftKey } = event.nativeEvent as TextInputKeyPressEventData & { shiftKey?: boolean };
    if (key === "Escape") {
      findController.close();
      return;
    }
    if (key === "Enter" && Platform.OS === "web") {
      void findController.step(shiftKey ? -1 : 1);
    }
  };

  const none = find.result.count === 0;
  const label = findCountLabel(find.query, find.result, find.searching);
  return (
    <View
      style={[styles.bar, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }, (scheme === "dark" ? depthDark : depthLight).sheet]}
      accessibilityRole="search"
    >
      <Search size={14} strokeWidth={1.75} color={tokens.ink3} />
      <TextInput
        ref={inputRef}
        value={find.query}
        onChangeText={findController.setQuery}
        onKeyPress={onKeyPress}
        onSubmitEditing={Platform.OS === "web" ? undefined : () => void findController.step(1)}
        blurOnSubmit={false}
        placeholder="Find"
        placeholderTextColor={tokens.ink4}
        selectionColor={tokens.accent}
        autoCapitalize="none"
        autoCorrect={false}
        spellCheck={false}
        returnKeyType="search"
        accessibilityLabel="Find in this view"
        style={[styles.input, typeScale.body, { color: tokens.ink }]}
      />
      <Text
        style={[typeScale.monoMeta, tabularNums, styles.count, { color: find.query !== "" && none && !find.searching ? tokens.warn : tokens.ink3 }]}
        accessibilityLiveRegion="polite"
        numberOfLines={1}
      >
        {label}
      </Text>
      <BarButton label="Match case" active={find.caseSensitive} onPress={findController.toggleCase}>
        <Text style={[typeScale.monoMeta, { color: find.caseSensitive ? tokens.accent : tokens.ink3 }]}>Aa</Text>
      </BarButton>
      <BarButton label="Previous match" disabled={none} onPress={() => void findController.step(-1)}>
        <ChevronUp size={16} strokeWidth={1.75} color={tokens.ink2} />
      </BarButton>
      <BarButton label="Next match" disabled={none} onPress={() => void findController.step(1)}>
        <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink2} />
      </BarButton>
      <BarButton label="Close find" onPress={findController.close}>
        <X size={16} strokeWidth={1.75} color={tokens.ink2} />
      </BarButton>
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    position: "absolute",
    top: space.space8,
    right: space.space12,
    left: space.space12,
    maxWidth: 440,
    alignSelf: "flex-end",
    zIndex: 20,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingLeft: space.space12,
    paddingRight: space.space4,
    minHeight: 40,
    borderWidth: 1,
    borderRadius: radii.radius12,
  },
  input: { flex: 1, minWidth: 80, paddingVertical: space.space4, paddingHorizontal: space.space4 },
  count: { minWidth: 56, textAlign: "right", paddingHorizontal: space.space4 },
  button: {
    minWidth: 36,
    minHeight: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radii.radius8,
  },
});
