// The full primitive library (BUILD_PLAN §3 / §7 Batch 0). Every screen composes these —
// no screen hand-rolls headers, raw hex, or unbounded lists (UI_RULES.md §1-§2).
//
// Notes for later-batch workers:
// - No icon library is pinned in BUILD_PLAN §3, so primitives here use plain text glyphs
//   (●/○/◐, "+", "×", chevrons) instead of vector icons. If a screen needs real icons,
//   flag it before adding @expo/vector-icons or similar (UI_RULES.md #24).
// - Every color reference here goes through `theme.colors` (never a raw hex literal) or a
//   NativeWind className using the token names from tailwind.config.js.
import React, { useCallback } from "react";
import {
  ActivityIndicator,
  FlatList,
  type FlatListProps,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  type PressableProps,
  RefreshControl,
  ScrollView,
  Text,
  TextInput,
  type TextInputProps,
  View,
  type ViewStyle,
} from "react-native";
import Animated from "react-native-reanimated";
import { SafeAreaView } from "react-native-safe-area-context";

import { useEntranceAnimation, usePressScale, usePulse } from "../lib/motion";
import { minTapTarget, theme } from "../lib/theme";

function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

export interface ScreenProps {
  children: React.ReactNode;
  /** Wraps children in a ScrollView. Set false when the body owns a BoundedList/FlatList. */
  scroll?: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
  keyboardAvoiding?: boolean;
  contentContainerClassName?: string;
  className?: string;
}

/**
 * One `Screen` per route (UI_RULES.md #1). Owns: safe-area insets, bg, horizontal gutter
 * (12px), and the scroll container / pull-to-refresh.
 */
export function Screen({
  children,
  scroll = true,
  refreshing,
  onRefresh,
  keyboardAvoiding = false,
  contentContainerClassName,
  className,
}: ScreenProps) {
  const body = scroll ? (
    <ScrollView
      className={cn("flex-1", className)}
      contentContainerClassName={cn("px-12 pb-16", contentContainerClassName)}
      refreshControl={
        onRefresh ? (
          <RefreshControl
            refreshing={refreshing ?? false}
            onRefresh={onRefresh}
            tintColor={theme.colors.dim}
          />
        ) : undefined
      }
      keyboardShouldPersistTaps="handled"
    >
      {children}
    </ScrollView>
  ) : (
    <View className={cn("flex-1 px-12", className)}>{children}</View>
  );

  return (
    <SafeAreaView className="flex-1 bg-bg" edges={["left", "right", "bottom"]}>
      {keyboardAvoiding ? (
        <KeyboardAvoidingView
          className="flex-1"
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          keyboardVerticalOffset={Platform.OS === "ios" ? 8 : 0}
        >
          {body}
        </KeyboardAvoidingView>
      ) : (
        body
      )}
    </SafeAreaView>
  );
}

// ---------------------------------------------------------------------------
// Card / Tile
// ---------------------------------------------------------------------------

export interface CardProps {
  children: React.ReactNode;
  onPress?: () => void;
  /** "default" = radius 8 (rows/cards); "feature" = radius 10 (overlay/plan/diff cards). */
  variant?: "default" | "feature";
  className?: string;
}

export function Card({ children, onPress, variant = "default", className }: CardProps) {
  const { style, onPressIn, onPressOut } = usePressScale();
  const radiusClass = variant === "feature" ? "rounded-lg" : "rounded-md";

  if (!onPress) {
    return (
      <View
        className={cn(
          "bg-panel border border-borderSoft px-10 py-8",
          radiusClass,
          className,
        )}
      >
        {children}
      </View>
    );
  }

  return (
    <Animated.View style={style}>
      <Pressable
        onPress={onPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        className={cn(
          "bg-panel border border-borderSoft px-10 py-8",
          radiusClass,
          className,
        )}
        style={{ minHeight: minTapTarget }}
      >
        {children}
      </Pressable>
    </Animated.View>
  );
}

export interface TileProps {
  title: string;
  subtitle?: string;
  right?: React.ReactNode;
  onPress?: () => void;
  className?: string;
}

export function Tile({ title, subtitle, right, onPress, className }: TileProps) {
  return (
    <Card onPress={onPress} className={className}>
      <View className="flex-row items-center gap-8">
        <View className="flex-1">
          <Text numberOfLines={1} className="text-ink text-[15px] font-semibold">
            {title}
          </Text>
          {subtitle ? (
            <Text numberOfLines={1} className="text-dim text-[12px] mt-2">
              {subtitle}
            </Text>
          ) : null}
        </View>
        {right}
      </View>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// ListRow
// ---------------------------------------------------------------------------

export interface ListRowProps {
  title: string;
  subtitle?: string;
  /** cwd/branch tails ellipsize head; titles ellipsize tail (UI_RULES.md #5). */
  subtitleEllipsize?: "head" | "tail";
  left?: React.ReactNode;
  right?: React.ReactNode;
  onPress?: () => void;
  onLongPress?: () => void;
  disabled?: boolean;
  className?: string;
}

function ListRowBase({
  title,
  subtitle,
  subtitleEllipsize = "tail",
  left,
  right,
  onPress,
  onLongPress,
  disabled,
  className,
}: ListRowProps) {
  const { style, onPressIn, onPressOut } = usePressScale();

  const content = (
    <View
      className={cn(
        "flex-row items-center gap-8 px-10 py-8 border-b border-histBorder",
        className,
      )}
      style={{ minHeight: minTapTarget }}
    >
      {left}
      <View className="flex-1">
        <Text numberOfLines={1} className="text-ink text-[15px]">
          {title}
        </Text>
        {subtitle ? (
          <Text
            numberOfLines={1}
            ellipsizeMode={subtitleEllipsize}
            className="text-dim text-[12px] mt-2"
          >
            {subtitle}
          </Text>
        ) : null}
      </View>
      {right}
    </View>
  );

  if (!onPress && !onLongPress) return content;

  return (
    <Animated.View style={style}>
      <Pressable
        onPress={onPress}
        onLongPress={onLongPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        disabled={disabled}
      >
        {content}
      </Pressable>
    </Animated.View>
  );
}

export const ListRow = React.memo(ListRowBase);

// ---------------------------------------------------------------------------
// StatCard / Metric
// ---------------------------------------------------------------------------

export type Tone = "ink" | "ok" | "no" | "accent" | "dim";

const toneClass: Record<Tone, string> = {
  ink: "text-ink",
  ok: "text-ok",
  no: "text-no",
  accent: "text-accent",
  dim: "text-dim",
};

export interface StatCardProps {
  label: string;
  value: string;
  tone?: Tone;
  className?: string;
}

export function StatCard({ label, value, tone = "ink", className }: StatCardProps) {
  return (
    <Card className={cn("gap-4", className)}>
      <Text className="text-dim text-[11px] font-semibold uppercase tracking-[0.5px]">
        {label}
      </Text>
      <Text
        className={cn("text-[16px] font-bold", toneClass[tone])}
        style={{ fontVariant: ["tabular-nums"] }}
      >
        {value}
      </Text>
    </Card>
  );
}

export interface MetricProps {
  value: number;
  /** "cost" => $ + 4dp under $1 / 2dp above (UI_RULES #11); "int" => integer; "raw" => as-is string. */
  format?: "cost" | "int" | "raw";
  tone?: Tone;
  className?: string;
}

export function formatMetric(value: number, format: MetricProps["format"] = "raw"): string {
  if (format === "cost") {
    return `$${value < 1 ? value.toFixed(4) : value.toFixed(2)}`;
  }
  if (format === "int") {
    return Math.round(value).toLocaleString();
  }
  return String(value);
}

export function Metric({ value, format = "raw", tone = "ink", className }: MetricProps) {
  return (
    <Text
      className={cn("text-[13px]", toneClass[tone], className)}
      style={{ fontVariant: ["tabular-nums"] }}
    >
      {formatMetric(value, format)}
    </Text>
  );
}

// ---------------------------------------------------------------------------
// SectionTitle
// ---------------------------------------------------------------------------

export function SectionTitle({
  children,
  className,
}: {
  children: string;
  className?: string;
}) {
  return (
    <Text
      className={cn(
        "text-dim text-[11px] font-bold uppercase tracking-[0.5px] mb-6",
        className,
      )}
    >
      {children}
    </Text>
  );
}

// ---------------------------------------------------------------------------
// Badge
// ---------------------------------------------------------------------------

export type BadgeTone = "default" | "ok" | "no" | "accent" | "warn";

const badgeToneClass: Record<BadgeTone, string> = {
  default: "bg-chipBg text-ink",
  ok: "bg-chipBg text-ok",
  no: "bg-pubBg text-pubInk",
  accent: "bg-selBg text-accent",
  warn: "bg-bannerBg text-bannerInk",
};

export function Badge({ label, tone = "default" }: { label: string; tone?: BadgeTone }) {
  const [bgClass, textClass] = badgeToneClass[tone].split(" ");
  return (
    <View className={cn("rounded-sm px-6 py-2", bgClass)}>
      <Text className={cn("text-[12px] font-semibold", textClass)} numberOfLines={1}>
        {label}
      </Text>
    </View>
  );
}

// ---------------------------------------------------------------------------
// Chip
// ---------------------------------------------------------------------------

export interface ChipProps {
  label: string;
  selected?: boolean;
  onPress?: () => void;
  tone?: "default" | "danger";
  disabled?: boolean;
}

export function Chip({ label, selected, onPress, tone = "default", disabled }: ChipProps) {
  const { style, onPressIn, onPressOut } = usePressScale();
  return (
    <Animated.View style={style}>
      <Pressable
        onPress={onPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        disabled={disabled}
        hitSlop={8}
        className={cn(
          "rounded-pill border px-16 py-11 items-center justify-center",
          selected ? "bg-selBg border-accent" : "bg-chipBg border-border",
          disabled && "opacity-50",
        )}
        style={{ minHeight: minTapTarget }}
      >
        <Text
          numberOfLines={1}
          className={cn(
            "text-[13px] font-medium",
            tone === "danger" ? "text-no" : selected ? "text-accent" : "text-ink",
          )}
        >
          {label}
        </Text>
      </Pressable>
    </Animated.View>
  );
}

// ---------------------------------------------------------------------------
// Segmented
// ---------------------------------------------------------------------------

export interface SegmentedOption {
  key: string;
  label: string;
  count?: number;
  dot?: boolean;
}

export interface SegmentedProps {
  options: SegmentedOption[];
  value: string;
  onChange: (key: string) => void;
}

export function Segmented({ options, value, onChange }: SegmentedProps) {
  return (
    <View className="flex-row bg-panelDeep rounded-md p-2 gap-2">
      {options.map((opt) => {
        const active = opt.key === value;
        return (
          <Pressable
            key={opt.key}
            onPress={() => onChange(opt.key)}
            className={cn(
              "flex-1 flex-row items-center justify-center rounded-sm py-8 gap-4",
              active && "bg-panel",
            )}
            style={{ minHeight: minTapTarget - 8 }}
          >
            <Text
              numberOfLines={1}
              className={cn(
                "text-[13px] font-semibold",
                active ? "text-accent" : "text-dim",
              )}
            >
              {opt.label}
              {opt.count != null ? ` (${opt.count})` : ""}
            </Text>
            {opt.dot ? <View className="w-6 h-6 rounded-full bg-accent" /> : null}
          </Pressable>
        );
      })}
    </View>
  );
}

// ---------------------------------------------------------------------------
// SearchInput
// ---------------------------------------------------------------------------

export interface SearchInputProps
  extends Omit<TextInputProps, "style" | "value" | "onChangeText"> {
  value: string;
  onChangeText: (text: string) => void;
  className?: string;
}

export function SearchInput({
  value,
  onChangeText,
  placeholder,
  className,
  ...rest
}: SearchInputProps) {
  return (
    <TextInput
      value={value}
      onChangeText={onChangeText}
      placeholder={placeholder}
      placeholderTextColor={theme.colors.dim}
      className={cn(
        "bg-panel border border-border rounded-md px-10 text-ink text-[15px]",
        className,
      )}
      style={{ minHeight: minTapTarget }}
      {...rest}
    />
  );
}

// ---------------------------------------------------------------------------
// PrimaryButton / ConfirmButton
// ---------------------------------------------------------------------------

export interface PrimaryButtonProps {
  label: string;
  onPress: () => void;
  loading?: boolean;
  disabled?: boolean;
  fullWidth?: boolean;
  className?: string;
}

export function PrimaryButton({
  label,
  onPress,
  loading,
  disabled,
  fullWidth = true,
  className,
}: PrimaryButtonProps) {
  const { style, onPressIn, onPressOut } = usePressScale();
  const isDisabled = disabled || loading;
  return (
    <Animated.View style={[style, fullWidth && { alignSelf: "stretch" }]}>
      <Pressable
        onPress={onPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        disabled={isDisabled}
        className={cn(
          "bg-accent rounded-md items-center justify-center flex-row gap-8 px-16",
          isDisabled && "opacity-50",
          className,
        )}
        style={{ minHeight: minTapTarget }}
      >
        {loading ? <ActivityIndicator color={theme.colors.panel} /> : null}
        <Text className="text-panel text-[15px] font-bold">{label}</Text>
      </Pressable>
    </Animated.View>
  );
}

export interface ConfirmButtonProps {
  label: string;
  tone: "ok" | "no";
  onPress: () => void;
  loading?: boolean;
  disabled?: boolean;
  className?: string;
}

/**
 * Allow/Deny + destructive confirm. Always fires a light haptic (UI_RULES.md #19, #37).
 * Allow = `ok` bg, Deny/destructive = `no` bg, both with `panel`-tone text (UI_RULES.md #10).
 */
export function ConfirmButton({
  label,
  tone,
  onPress,
  loading,
  disabled,
  className,
}: ConfirmButtonProps) {
  const { style, onPressIn, onPressOut } = usePressScale();
  const isDisabled = disabled || loading;

  const handlePress = useCallback(() => {
    if (Platform.OS !== "web") {
      import("expo-haptics")
        .then((Haptics) =>
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light),
        )
        .catch(() => {});
    }
    onPress();
  }, [onPress]);

  return (
    <Animated.View style={style}>
      <Pressable
        onPress={handlePress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        disabled={isDisabled}
        className={cn(
          "rounded-md items-center justify-center flex-row gap-8 px-16",
          tone === "ok" ? "bg-ok" : "bg-no",
          isDisabled && "opacity-50",
          className,
        )}
        style={{ minHeight: minTapTarget }}
      >
        {loading ? <ActivityIndicator color={theme.colors.panel} /> : null}
        <Text className="text-panel text-[15px] font-bold">{label}</Text>
      </Pressable>
    </Animated.View>
  );
}

// ---------------------------------------------------------------------------
// FAB
// ---------------------------------------------------------------------------

export interface FABProps {
  onPress: () => void;
  label?: string;
  glyph?: string;
}

export function FAB({ onPress, label, glyph = "+" }: FABProps) {
  const { style, onPressIn, onPressOut } = usePressScale();
  return (
    <Animated.View
      style={[style, { position: "absolute", right: 16, bottom: 16 }]}
    >
      <Pressable
        onPress={onPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        accessibilityLabel={label ?? "New"}
        className="bg-accent rounded-full items-center justify-center px-16 flex-row gap-6"
        style={{ minHeight: 56, minWidth: label ? undefined : 56 }}
      >
        <Text className="text-panel text-[20px] font-bold">{glyph}</Text>
        {label ? (
          <Text className="text-panel text-[15px] font-bold">{label}</Text>
        ) : null}
      </Pressable>
    </Animated.View>
  );
}

// ---------------------------------------------------------------------------
// EmptyState / Loading / ErrorText
// ---------------------------------------------------------------------------

export interface EmptyStateProps {
  glyph?: string;
  title: string;
  action?: { label: string; onPress: () => void };
}

export function EmptyState({ glyph = "◌", title, action }: EmptyStateProps) {
  return (
    <View className="items-center justify-center gap-10 px-16 py-16">
      <Text className="text-dim text-[24px]">{glyph}</Text>
      <Text className="text-dim text-[14px] text-center">{title}</Text>
      {action ? (
        <PrimaryButton label={action.label} onPress={action.onPress} fullWidth={false} />
      ) : null}
    </View>
  );
}

export function Loading({ label }: { label?: string }) {
  return (
    <View className="items-center justify-center gap-8 py-16">
      <ActivityIndicator color={theme.colors.dim} />
      {label ? <Text className="text-dim text-[12px]">{label}</Text> : null}
    </View>
  );
}

export interface ErrorTextProps {
  message: string;
  onRetry?: () => void;
}

export function ErrorText({ message, onRetry }: ErrorTextProps) {
  return (
    <View className="items-center justify-center gap-10 px-16 py-16">
      <Text className="text-no text-[14px] text-center">{message}</Text>
      {onRetry ? <PrimaryButton label="Retry" onPress={onRetry} fullWidth={false} /> : null}
    </View>
  );
}

// ---------------------------------------------------------------------------
// BoundedList
// ---------------------------------------------------------------------------

export interface BoundedListProps<T>
  extends Omit<
    FlatListProps<T>,
    "data" | "renderItem" | "keyExtractor" | "ListEmptyComponent"
  > {
  data: T[];
  keyExtractor: (item: T) => string;
  renderItem: (info: { item: T; index: number }) => React.ReactElement;
  /** Mandatory — never a bare empty list (UI_RULES.md #7, #13). */
  ListEmptyComponent: React.ReactElement;
  loading?: boolean;
}

/**
 * The ONLY sanctioned way to render a list (UI_RULES.md #7). Never `.map()` an unbounded
 * array in a ScrollView. Virtualized with sane perf defaults (UI_RULES.md #26-27).
 */
export function BoundedList<T>({
  data,
  keyExtractor,
  renderItem,
  ListEmptyComponent,
  loading,
  refreshing,
  onRefresh,
  ...rest
}: BoundedListProps<T>) {
  const keyExtractorStable = useCallback(
    (item: T) => keyExtractor(item),
    [keyExtractor],
  );
  const renderItemStable = useCallback(
    ({ item, index }: { item: T; index: number }) => renderItem({ item, index }),
    [renderItem],
  );

  if (loading && data.length === 0) {
    return <Loading />;
  }

  return (
    <FlatList
      data={data}
      keyExtractor={keyExtractorStable}
      renderItem={renderItemStable}
      ListEmptyComponent={ListEmptyComponent}
      refreshControl={
        onRefresh ? (
          <RefreshControl
            refreshing={refreshing ?? false}
            onRefresh={onRefresh}
            tintColor={theme.colors.dim}
          />
        ) : undefined
      }
      removeClippedSubviews={Platform.OS !== "web"}
      maxToRenderPerBatch={12}
      windowSize={7}
      initialNumToRender={12}
      {...rest}
    />
  );
}

// ---------------------------------------------------------------------------
// StatusDot — busy/waiting/idle pulse (used by Fleet rows + session header)
// ---------------------------------------------------------------------------

export type StatusDotState = "busy" | "waiting" | "idle" | "idle-past";

const statusDotColor: Record<StatusDotState, string> = {
  busy: "bg-accent",
  waiting: "bg-no",
  idle: "bg-ok",
  "idle-past": "bg-dim",
};

export function StatusDot({ state }: { state: StatusDotState }) {
  const pulseStyle = usePulse(state === "busy" ? "busy" : "waiting");
  const animated = state === "busy" || state === "waiting";
  const dot = <View className={cn("w-8 h-8 rounded-full", statusDotColor[state])} />;
  if (!animated) return dot;
  return <Animated.View style={pulseStyle}>{dot}</Animated.View>;
}

// ---------------------------------------------------------------------------
// EntranceView — wraps a row/card with the shared staggered entrance (motion.ts)
// ---------------------------------------------------------------------------

export function EntranceView({
  index,
  children,
  style,
}: {
  index: number;
  children: React.ReactNode;
  style?: ViewStyle;
}) {
  const entranceStyle = useEntranceAnimation(index);
  return <Animated.View style={[entranceStyle, style]}>{children}</Animated.View>;
}

export type { PressableProps };
