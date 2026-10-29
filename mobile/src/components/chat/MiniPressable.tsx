// A small bare button with the keyboard focus ring and hover wash the larger ds controls have,
// for icon-sized actions inside dense composer rows (queue strip, attachment tiles, hint toggle).
import React, { useState } from "react";
import { Pressable, type PressableProps, type StyleProp, type ViewStyle } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";

export interface MiniPressableProps extends Omit<PressableProps, "style"> {
  style?: StyleProp<ViewStyle>;
  /** Paint a wash behind the control while hovered or focused. */
  wash?: boolean;
  /** Border shown while the control is not focused; transparent by default. */
  idleBorderColor?: string;
}

export function MiniPressable({ style, wash = true, idleBorderColor = "transparent", children, disabled, ...rest }: MiniPressableProps) {
  const tokens = useTokens();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  return (
    <Pressable
      {...rest}
      disabled={disabled}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      style={[
        { borderWidth: 1, borderColor: focused ? tokens.accent : idleBorderColor },
        wash && hovered && !disabled ? { backgroundColor: tokens.bg3 } : null,
        style,
      ]}
    >
      {children}
    </Pressable>
  );
}
