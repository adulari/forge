# Forge TUI Design System

## Hierarchy

1. Conversation and composer
2. Current permission, error, or active-work summary
3. Session context
4. On-demand operational views

The resting chat never shows detailed task, agent, quota, or workflow panels. `Ctrl+O` expands
current work detail; dedicated workflow, activity, usage, mesh, configuration, and session views
retain their full data.

## Navigation

- `Ctrl+K`: searchable command center for commands, skills, custom commands, settings, and views.
- `/`: inline slash completion for users who know the command they want.
- `F1` or `?`: keyboard reference.
- `Esc`: closes the current surface before leaving the session.

## Visual Rules

- Use Forge orange for primary focus and blue/cyan for active technical state.
- Preserve dark terminal surfaces but keep body text readable and use semantic success, warning,
  and error colors with text or glyph alternatives.
- Prefer one-line summaries over persistent panels. Put density in dedicated operational views.
- Use rounded borders only for genuinely framed controls such as the composer and command center.
- Keep keyboard hints brief and contextual; do not make the UI explain itself with dense prose.
