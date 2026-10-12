//! The slim line shapes pass one of the transcript scan deserializes: just the ids and links,
//! never the (possibly huge) message bodies.

use serde::Deserialize;

#[derive(Deserialize, Default)]
pub(super) struct SlimMessage {
    pub(super) id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) content: Option<ToolIds>,
}

#[derive(Deserialize)]
pub(super) struct SlimBlock {
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    pub(super) id: Option<String>,
    pub(super) tool_use_id: Option<String>,
}

/// The tool ids in a message's content, read without keeping the (possibly huge) bodies: a string
/// content has none, a block list yields `(tool_use ids, tool_result ids)`.
#[derive(Default)]
pub(super) struct ToolIds {
    pub(super) uses: Vec<String>,
    pub(super) results: Vec<String>,
}

impl<'de> Deserialize<'de> for ToolIds {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = ToolIds;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("message content")
            }
            fn visit_str<E>(self, _: &str) -> std::result::Result<ToolIds, E> {
                Ok(ToolIds::default())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<ToolIds, A::Error> {
                let mut ids = ToolIds::default();
                while let Some(b) = seq.next_element::<SlimBlock>()? {
                    match (b.kind.as_deref(), b.id, b.tool_use_id) {
                        (Some("tool_use"), Some(id), _) => ids.uses.push(id),
                        (Some("tool_result"), _, Some(id)) => ids.results.push(id),
                        _ => {}
                    }
                }
                Ok(ids)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<ToolIds, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(ToolIds::default())
            }
            fn visit_unit<E>(self) -> std::result::Result<ToolIds, E> {
                Ok(ToolIds::default())
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Deserialize)]
pub(super) struct SlimSegment {
    #[serde(rename = "headUuid")]
    pub(super) head: Option<String>,
    #[serde(rename = "tailUuid")]
    pub(super) tail: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SlimCompact {
    #[serde(rename = "preservedSegment")]
    pub(super) segment: Option<SlimSegment>,
}

#[derive(Deserialize)]
pub(super) struct Slim {
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    pub(super) subtype: Option<String>,
    pub(super) uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    pub(super) parent: Option<String>,
    #[serde(rename = "logicalParentUuid")]
    pub(super) logical_parent: Option<String>,
    #[serde(rename = "isSidechain", default)]
    pub(super) sidechain: bool,
    #[serde(rename = "isCompactSummary", default)]
    pub(super) compact_summary: bool,
    pub(super) cwd: Option<String>,
    #[serde(rename = "customTitle")]
    pub(super) custom_title: Option<String>,
    #[serde(rename = "aiTitle")]
    pub(super) ai_title: Option<String>,
    pub(super) message: Option<SlimMessage>,
    #[serde(rename = "compactMetadata")]
    pub(super) compact: Option<SlimCompact>,
}
