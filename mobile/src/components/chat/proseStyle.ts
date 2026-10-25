// Chat prose size. `type.body` (13/21) is the dense chrome size; a conversation read on a phone
// at arm's length needs more, so chat messages use 15/23 on phone widths and 14/22 elsewhere.
import type { TextStyle } from "react-native";

import { useBreakpoint } from "../../theme/useBreakpoint";

const COMPACT: TextStyle = { fontSize: 15, lineHeight: 23 };
const REGULAR: TextStyle = { fontSize: 14, lineHeight: 22 };

export function useChatProseStyle(): TextStyle {
  return useBreakpoint().isCompact ? COMPACT : REGULAR;
}
