// Hearth core rule 6: the task composer replaces every "new session" affordance
// across every surface — bottom-floating on mobile Fleet, top of rail on
// desktop/web, centered in the web empty state. Callers position it; this
// component only renders the pill itself. Pill radius 999, "Describe a task to
// forge…" placeholder, 38px ember send disc. Web+native via RN TextInput/Pressable.
import { ArrowUp } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, TextInput, View, type StyleProp, type ViewStyle } from "react-native";
import Animated from "react-native-reanimated";

import { useStrike } from "../../theme/motion";
import { useTheme } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, shadowStyle, space, tapTarget } from "../../theme/tokens";
import { type, webInputTextStyle } from "../../theme/typography";

export interface TaskComposerProps {
  value: string;
  onChangeText: (text: string) => void;
  onSubmit: (text: string) => void;
  placeholder?: string;
  autoFocus?: boolean;
  /** Denser pill for tight rails (desktop sidebar top-of-rail). Default is the full
   * mobile Fleet size. */
  compact?: boolean;
  disabled?: boolean;
  testID?: string;
  style?: StyleProp<ViewStyle>;
}

const DISC_SIZE = 38;
const DISC_SIZE_COMPACT = 32;
// 38px is below the 44px minimum hit target (BUILD rules) — hitSlop makes up the
// difference without changing the visual disc size the spec calls for.
const DISC_HIT_SLOP = (tapTarget - DISC_SIZE) / 2;

export function TaskComposer({
  value,
  onChangeText,
  onSubmit,
  placeholder = "Describe a task to forge…",
  autoFocus = false,
  compact = false,
  disabled = false,
  testID,
  style,
}: TaskComposerProps) {
  const { scheme, tokens } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;
  const strike = useStrike();
  const [focused, setFocused] = useState(false);

  const canSend = value.trim().length > 0 && !disabled;
  const commit = () => {
    if (!canSend) return;
    onSubmit(value);
  };

  return (
    <View
      style={[
        styles.pill,
        compact ? styles.compact : styles.regular,
        {
          backgroundColor: tokens.bg2,
          borderColor: focused ? tokens.accent : tokens.borderStrong,
        },
        shadowStyle(depth.sheet),
        style,
      ]}
      testID={testID}
    >
      <TextInput
        value={value}
        onChangeText={onChangeText}
        onSubmitEditing={commit}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        placeholder={placeholder}
        placeholderTextColor={tokens.ink3}
        autoFocus={autoFocus}
        autoCapitalize="none"
        autoCorrect={false}
        editable={!disabled}
        returnKeyType="send"
        // Kindle caret (Forgework motion set): the composer's own text cursor is
        // accent-tinted rather than the platform default.
        cursorColor={tokens.accent}
        selectionColor={tokens.accent}
        style={[type.body, webInputTextStyle, styles.input, { color: tokens.ink }]}
        accessibilityLabel={placeholder}
        testID={testID ? `${testID}-input` : undefined}
      />
      <Animated.View style={strike.style}>
        <Pressable
          onPress={commit}
          onPressIn={disabled ? undefined : strike.onPressIn}
          onPressOut={disabled ? undefined : strike.onPressOut}
          disabled={!canSend}
          hitSlop={DISC_HIT_SLOP}
          accessibilityRole="button"
          accessibilityLabel="forge session"
          accessibilityState={{ disabled: !canSend }}
          testID={testID ? `${testID}-send` : undefined}
          style={[
            styles.disc,
            compact ? styles.discCompact : undefined,
            { backgroundColor: canSend ? tokens.accent : tokens.bg3 },
          ]}
        >
          <ArrowUp size={15} strokeWidth={2.4} color={canSend ? tokens.onAccent : tokens.ink4} />
        </Pressable>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  pill: {
    flexDirection: "row",
    alignItems: "center",
    borderWidth: 1,
    borderRadius: radii.radiusPill,
    paddingLeft: space.space16 + 2,
    paddingRight: space.space8,
    gap: space.space8,
  },
  regular: { minHeight: 52 },
  compact: { minHeight: 44 },
  input: { flex: 1, paddingVertical: space.space8 },
  disc: {
    width: DISC_SIZE,
    height: DISC_SIZE,
    borderRadius: radii.radiusPill,
    alignItems: "center",
    justifyContent: "center",
  },
  discCompact: { width: DISC_SIZE_COMPACT, height: DISC_SIZE_COMPACT },
});
