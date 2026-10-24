//! Which pinned models are known to be uncallable, so a surface can hold a prompt back instead of
//! failing the turn it would start.

use crate::Session;

impl Session {
    /// Record the pinned ids startup found no provider able to serve at all (composition root).
    pub fn set_unavailable_models(&mut self, models: Vec<String>) {
        self.unavailable_models = models;
    }

    /// The pinned model when every member of the pin is known to be uncallable: a prompt sent now
    /// would only fail the turn, so the surface can keep it in the composer and point at `/model`.
    pub fn unavailable_pin(&self) -> Option<&str> {
        let pin = self.pinned_model.as_deref()?;
        pin.iter()
            .all(|id| self.unavailable_models.contains(id))
            .then(|| pin.first().map(String::as_str))
            .flatten()
    }
}
