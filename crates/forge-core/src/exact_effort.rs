//! The session's EXACT reasoning rung (`/effort exact <level>`).
//!
//! Its own owner rather than another pair of accessors in `session_controls`: that file had eleven
//! lines of headroom under the file-size guard, and spending them here would have left the next
//! change with none. The concept is also self-contained — the field is read in exactly one place,
//! the post-routing backstop in `lib.rs`.
//!
//! Distinct from the effort PIN next door, and the distinction is the whole point. The pin is a
//! CEILING: the mesh then picks the best-value rung at or under it from the model's MEASURED
//! benchmark ladder. That ladder holds only the rungs somebody rated, which is not the set the
//! provider offers — Kimi Code serves low/high/max but is rated only at low and max, so no ceiling
//! can ask for its `high`, and asking for `high` actually resolves DOWN to `low`. This is an
//! INSTRUCTION: the rung is sent as-is, still resolved against the routed model's own provider
//! ladder so it can never name a rung that does not exist.

use super::*;

impl Session {
    /// Set (or clear) the in-session exact rung. Not persisted, unlike the effort pin: this is an
    /// ad-hoc override whose durable form is `[model_effort]` in config, and giving it a session
    /// column would mean a schema migration for something deliberately short-lived.
    pub fn set_exact_effort(&mut self, e: Option<EffortLevel>) {
        self.exact_effort = e;
    }

    /// The exact rung this session forces, if any. `None` = the mesh's own choice stands.
    pub fn exact_effort(&self) -> Option<EffortLevel> {
        self.exact_effort
    }
}
