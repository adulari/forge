//! Public Forge Anywhere v1 protocol.
//!
//! This crate is deliberately service-independent: it defines the encrypted binary envelope,
//! key derivation, recovery material, sync records, and the explicit daemon bridge allowlist.
//! The private hosted service consumes the published bytes and golden fixtures without linking
//! this AGPL crate.

pub mod bridge;
pub mod capsule;
pub mod command;
pub mod crypto;
pub mod envelope;
pub mod keys;
pub mod share;
pub mod sync;

pub use capsule::{
    CapsuleAcknowledgeRequest, CapsuleAcknowledgement, CapsuleClaim, CapsuleCompletion, CapsuleId,
    CapsuleOutcome, CapsuleReservation, CapsuleReserveRequest, CapsuleStatus, PendingCapsule,
    PendingCapsuleList, CAPSULE_EXPIRY_SECONDS, CAPSULE_FLAG_ACCEPTED, CAPSULE_VERSION,
    MAX_CAPSULE_ENVELOPE_BYTES,
};
pub use command::{
    CommandAcknowledgement, CommandEnqueueResponse, CommandErrorCode, CommandId,
    CommandMetadataError, CommandResult, QueuedCommandList, QueuedCommandMetadata,
    COMMAND_ENQUEUE_VERSION, COMMAND_EXPIRY_MS, COMMAND_LIST_VERSION, MAX_COMMAND_ENVELOPE_BYTES,
};
pub use envelope::{
    Envelope, EnvelopeError, EnvelopeKind, EnvelopeMetadata, RecipientKind, MAGIC, VERSION,
};
pub use share::{
    ReplayShare, ShareCompletion, ShareId, ShareReservation, ShareReserveRequest,
    ANYWHERE_OBJECT_MEDIA_TYPE, MAX_SHARE_ENVELOPE_BYTES, SHARE_EXPIRY_SECONDS, SHARE_VERSION,
};

/// An opaque 128-bit account identifier visible to the relay for routing.
pub type AccountId = [u8; 16];

/// An opaque 128-bit device identifier visible to the relay for replay protection.
pub type DeviceId = [u8; 16];

/// An opaque 128-bit recipient identifier.
pub type RecipientId = [u8; 16];
