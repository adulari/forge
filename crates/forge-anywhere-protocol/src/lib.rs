//! Public Forge Anywhere v1 protocol.
//!
//! This crate is deliberately service-independent: it defines the encrypted binary envelope,
//! key derivation, recovery material, sync records, and the explicit daemon bridge allowlist.
//! The private hosted service consumes the published bytes and golden fixtures without linking
//! this AGPL crate.

pub mod bridge;
pub mod command;
pub mod crypto;
pub mod envelope;
pub mod keys;
pub mod sync;

pub use command::{
    CommandAcknowledgement, CommandErrorCode, CommandId, CommandMetadataError, CommandResult,
    QueuedCommandList, QueuedCommandMetadata, COMMAND_EXPIRY_MS, MAX_COMMAND_ENVELOPE_BYTES,
};
pub use envelope::{
    Envelope, EnvelopeError, EnvelopeKind, EnvelopeMetadata, RecipientKind, MAGIC, VERSION,
};

/// An opaque 128-bit account identifier visible to the relay for routing.
pub type AccountId = [u8; 16];

/// An opaque 128-bit device identifier visible to the relay for replay protection.
pub type DeviceId = [u8; 16];

/// An opaque 128-bit recipient identifier.
pub type RecipientId = [u8; 16];
