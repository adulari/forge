// STUB(B2): placeholder chat route. B3/T3.2 replaces this with the real timeline + composer.
import { useLocalSearchParams } from "expo-router";
import { Text } from "react-native";

import { Screen } from "../../../components/ds/Screen";
import { type } from "../../../theme/typography";
import { useTokens } from "../../../theme/ThemeProvider";

export default function SessionChat() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const tokens = useTokens();
  return (
    <Screen>
      <Text style={[type.body, { color: tokens.ink2 }]}>session {id}</Text>
    </Screen>
  );
}
