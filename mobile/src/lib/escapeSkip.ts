import { Platform } from "react-native";

/** Spread onto a container whose Escape key is its own (the terminal): the overlay stack leaves it alone. */
export const ESCAPE_SKIP_PROPS: object =
  Platform.OS === "web" ? { dataSet: { escapeSkip: "true" } } : {};
