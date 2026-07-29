# Code archaeology: Claude import policy boundary

`commands/import/claude.rs` owns only Claude settings, permission reconciliation, hook accounting, and MCP source/destination policy. The parent import command retains source selection, catalog migration, Cursor/Aider conversion, and shared filesystem copies.

The boundary preserves these invariants: project and user settings do not cross scope; CC permission aliases expand to every matching Forge tool; MCP import and runtime configuration retain scoped allowlist precedence; malformed or failed persistence is returned; and importer-owned permission blocks reconcile removals without touching unrelated configuration.
