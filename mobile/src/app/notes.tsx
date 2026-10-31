// Notes and to-dos per project, as a settings page (the phone's way to the Notes dock). Uses the
// same panel as the desktop dock; projects come from the host's recent list and its sessions.
import React, { useMemo, useState } from "react";

import { PageHeader } from "../components/ds/PageHeader";
import { Screen } from "../components/ds/Screen";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { NotesPanel } from "../components/notes/NotesPanel";
import { noteProjects } from "../components/notes/notesModel";
import { useProjects, useSessions } from "../lib/queries";
import { SettingsShell } from "./(tabs)/settings";

function NotesBody() {
  const catalog = useProjects().data;
  const sessionRows = useSessions().data;
  const projects = useMemo(
    () => noteProjects(sessionRows?.[0]?.cwd ?? null, catalog?.recent ?? [], sessionRows ?? []),
    [catalog, sessionRows],
  );
  const [picked, setPicked] = useState<string | null>(null);
  const project = picked && projects.some((p) => p.path === picked) ? picked : (projects[0]?.path ?? null);
  return (
    <Screen>
      <PageHeader title="Notes" subtitle="A scratchpad and to-dos for each project. Any to-do can start a thread." />
      <NotesPanel project={project} projects={projects} onProject={setPicked} inset={false} />
    </Screen>
  );
}

export default function NotesScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="notes">
        <NotesBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}
