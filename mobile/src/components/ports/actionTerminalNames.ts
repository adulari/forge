import { terminalTitle } from "../shell/terminalModel";
import { actionTerminalNames } from "./portsModel";
import type { ProjectAction } from "../../lib/apiProjectActions";

// Terminal ids are `act-<action id>`; the person should see the action's name instead. The names
// come from the actions query, so tabs adopted from the daemon's list pick them up as soon as it
// has answered once.
let names = new Map<string, string>();

export function rememberActionNames(actions: readonly ProjectAction[]): void {
  names = new Map([...names, ...actionTerminalNames(actions)]);
}

export function terminalLabel(terminalId: string): string {
  return names.get(terminalId) ?? terminalTitle(terminalId);
}
