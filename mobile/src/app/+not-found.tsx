import { Link } from "expo-router";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function NotFound() {
  const tokens = useTokens();
  return (
    <View style={[styles.root, { backgroundColor: tokens.bg1 }]}>
      <Text style={[type.title, { color: tokens.ink }]}>Page not found</Text>
      <Text style={[type.body, { color: tokens.ink2 }]}>That address does not match anything in Forge.</Text>
      <Link href="/" style={[type.body, { color: tokens.accent }]}>
        Go to Fleet
      </Link>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space12, padding: space.space16 },
});
