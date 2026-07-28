# Code archaeology: Config filesystem policy

## Intent

Config location and project-setup detection are stable filesystem policy,
separate from layered Figment configuration and secret resolution. Callers need
platform-global data for the Store and specific external CLI locations for
imports, while project initialization must be evaluated from an explicit
workspace rather than ambient process CWD.

## History and invariants

`fd2c54fd` introduced `project_initialization` and the Forge-owned auto-setup
marker. It deliberately treats guidance, agents, skills, commands, and config
as initialization evidence; an unsuccessful automatic setup is marked so it
cannot repeatedly spend quota in new sessions.

The extraction preserves:

- user config/data remain platform-global (`directories`); session history and
  budget never become project-local;
- external CLI paths stay optional when no home directory resolves;
- initialization and marker operations use their explicit `cwd` argument;
- all established root exports remain source compatible.

Focused Config tests cover initialization detection and the surrounding config
load behavior; no serialization, provider, Store, or permission behavior is
involved.
