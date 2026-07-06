// STUB(B2): minimal session route so the router graph + typed nav resolve during B2.
// B3/T3.1 replaces this with the real session shell (one socket, header, status strip,
// Segmented over Chat/Tasks/Agents/Review, banners).
import { Stack } from "expo-router";

export default function SessionLayout() {
  return <Stack screenOptions={{ headerShown: false }} />;
}
