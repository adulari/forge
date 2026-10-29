import { useAppShortcut } from "../../lib/shortcuts";
import { findController, useFindAvailable } from "../../lib/find/findController";

function FindShortcut() {
  useAppShortcut("find", () => {
    findController.request();
  });
  return null;
}

/** Claims Mod+F while some surface can be searched; with none, the browser's own find stays. */
export function FindHotkey() {
  return useFindAvailable() ? <FindShortcut /> : null;
}
