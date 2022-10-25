//! Blocklisting support.
//!
//! Blocked peers are prevent from interacting with the node through a variety of means.

use std::fmt::{self, Display, Formatter};

use casper_types::crypto;
use serde::Serialize;

use crate::{
    components::linear_chain::BlockSignatureError,
    types::{
        error::BlockHeadersBatchValidationError, BlockHash, BlockHeader, BlockHeadersBatchId, Tag,
    },
};

/// Reasons why a peer was blocked.
#[derive(Debug, Serialize)]
pub(crate) enum BlocklistJustification {
    /// Peer sent an item that was not parseable at all.
    SentBadItem { tag: Tag },
    /// Received a block with incorrect parent, which was specified beforehand.
    SentBlockWithWrongParent {
        /// The header that was received.
        received_header: BlockHeader,
        /// The parent that was expected.
        expected_parent_header: BlockHeader,
    },
    /// A block header or block was received without the appropriate finality signatures.
    MissingBlockSignatures {
        /// Received header/header of block.
        received_header: BlockHeader,
    },
    /// A peer sent a bogus block signature
    SentSignatureWithBogusValidator {
        /// The actual validation error.
        #[serde(skip_serializing)]
        error: BlockSignatureError,
    },
    /// A finality signature that was sent is cryptographically invalid.
    SentBadFinalitySignature {
        /// The actual cryptographic validation error.
        #[serde(skip_serializing)]
        error: crypto::Error,
    },
    /// A received batch of block headers was not valid.
    SentInvalidHeaderBatch {
        /// The ID of the received batch that did not validate.
        batch_id: BlockHeadersBatchId,
        /// The actual validation error.
        #[serde(skip_serializing)]
        error: BlockHeadersBatchValidationError,
    },
    /// A received block with signatures failed validation.
    SentBlockWithInvalidFinalitySignatures {
        /// The block header for which invalid signatures were received.
        block_header: BlockHeader,
        /// The block signature error.
        #[serde(skip_serializing)]
        error: BlockSignatureError,
    },
    /// A received block did not yield the expected execution results.
    SentBlockThatExecutedIncorrectly {
        /// The hash of the received block.
        block_hash: BlockHash,
    },
    /// An item received failed to validation.
    SentInvalidItem {
        /// The tag of the item received.
        tag: Tag,
        /// The ID of item, as a human-readable string.
        item_id: String,
        /// The validation error, as a human-readable string.
        error: String,
    },
    /// A serialized deploy was received that failed to deserialize.
    SentBadDeploy {
        /// The deserialization error.
        #[serde(skip_serializing)]
        error: bincode::Error,
    },
    /// A network address was received that should only be received via direct gossip.
    SentGossipedAddress,
}

impl Display for BlocklistJustification {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            BlocklistJustification::SentBadItem { tag } => {
                write!(f, "sent a {:?} item we couldn't parse", tag)
            }
            BlocklistJustification::SentBlockWithWrongParent {
                received_header,
                expected_parent_header,
            } => write!(
                f,
                "sent block (header: {:?}) with wrong parent ({:?})",
                received_header, expected_parent_header,
            ),
            BlocklistJustification::MissingBlockSignatures { received_header } => write!(
                f,
                "sent no block signatures along with header {:?}",
                received_header
            ),
            BlocklistJustification::SentSignatureWithBogusValidator { error } => {
                write!(f, "sent signature with bogus validator ({})", error)
            }
            BlocklistJustification::SentBadFinalitySignature { error } => write!(
                f,
                "sent cryptographically invalid finality signature: {}",
                error
            ),
            BlocklistJustification::SentInvalidHeaderBatch { batch_id, error } => write!(
                f,
                "sent block headers batch {} that failed validation: {}",
                batch_id, error
            ),
            BlocklistJustification::SentBlockWithInvalidFinalitySignatures {
                block_header,
                error,
            } => write!(
                f,
                "sent sent invalid finality signatures for block {:?}: {}",
                block_header, error
            ),
            BlocklistJustification::SentBlockThatExecutedIncorrectly { block_hash } => write!(
                f,
                "sent block {}, but the execution of the block did not yield the expected results",
                block_hash
            ),
            BlocklistJustification::SentInvalidItem {
                tag,
                item_id,
                error,
            } => write!(
                f,
                "sent item {} of type {:?} that was invalid: {}",
                item_id, tag, error
            ),
            BlocklistJustification::SentBadDeploy { error } => {
                write!(f, "sent a deploy that could not be deserialized: {}", error)
            }
            BlocklistJustification::SentGossipedAddress => {
                f.write_str("sent a network address via response, which is only ever gossiped")
            }
        }
    }
}
