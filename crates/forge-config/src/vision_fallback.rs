//! `[mesh] vision_fallback`: what a turn does when the model it landed on cannot read the images
//! the request carries (docs/features/image-delivery.md).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VisionFallback {
    /// Send the text only and say so. The default: nothing is spent or rerouted behind the user's
    /// back, and the model is never asked to guess at an image it was not given.
    #[default]
    Warn,
    /// A cheap vision-capable side call describes each image once; the description replaces the
    /// image for the text-only model.
    Describe,
    /// Route the request to the best vision-capable model, even when a model is pinned.
    Switch,
}

impl VisionFallback {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Describe => "describe",
            Self::Switch => "switch",
        }
    }
}
