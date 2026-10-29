import { useEffect, useState } from "react";
import { Platform } from "react-native";

const QUERY = "(hover: hover) and (pointer: fine)";

/** True on a web or desktop surface whose primary pointer can hover precisely; always false on a
 * phone, where hover affordances have no way to be reached. */
export function useFinePointer(): boolean {
  const [fine, setFine] = useState(() =>
    Platform.OS === "web" && typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia(QUERY).matches
      : false,
  );
  useEffect(() => {
    if (Platform.OS !== "web" || typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const media = window.matchMedia(QUERY);
    const update = () => setFine(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return fine;
}
