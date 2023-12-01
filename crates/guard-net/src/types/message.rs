#![allow(missing_docs)]
use std::{fmt::Debug, sync::Arc};

use alloy_rlp::{length_of_length, Decodable, Encodable, Header};
use guard_types::{
    consensus::{Commit, PreProposal, Proposal},
    primitive::Angstrom::Bundle,
    rpc::{
        SignedComposableLimitOrder, SignedComposableSearcherOrder, SignedLimitOrder,
        SignedSearcherOrder
    }
};
use reth_primitives::bytes::{Buf, BufMut};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{orders::GetComposableLimitOrders, version::StromVersion};
use crate::{
    errors::EthStreamError, ComposableLimitOrders, ComposableSearcherOrders,
    GetComposableSearcherOrders, GetLimitOrders, GetOrders, GetSearcherOrders, LimitOrders, Orders,
    SearcherOrders, Status
};

/// An `eth` protocol message, containing a message ID and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProtocolMessage {
    pub message_type: StromMessageID,
    pub message:      StromMessage
}

impl ProtocolMessage {
    /// Create a new ProtocolMessage from a message type and message rlp bytes.
    pub fn decode_message(_version: StromVersion, buf: &mut &[u8]) -> Result<Self, EthStreamError> {
        let message_type = StromMessageID::decode(buf)?;

        let message = match message_type {
            StromMessageID::Status => StromMessage::Status(Status::decode(buf)?),
            StromMessageID::PropagateOrder => {
                StromMessage::PropagateOrder(SignedLimitOrder::decode(buf)?)
            }
            StromMessageID::PrePropose => StromMessage::PrePropose(PreProposal::decode(buf)?),
            StromMessageID::Proposal => StromMessage::Proposal(Proposal::decode(buf)?),
            StromMessageID::Commit => StromMessage::Commit(Commit::decode(buf)?),
            StromMessageID::UserOrder => {
                StromMessage::UserOrders(RequestPair::<UserOrders>::decode(buf)?)
            }
            StromMessageID::LimitOrder => {
                StromMessage::LimitOrders(RequestPair::<LimitOrders>::decode(buf)?)
            }
            StromMessageID::SearcherOrder => {
                StromMessage::SearcherOrders(RequestPair::<SearcherOrders>::decode(buf)?)
            }
            StromMessageID::GetUserOrder => {
                StromMessage::GetUserOrders(RequestPair::<GetUsersOrders>::decode(buf)?)
            }
            StromMessageID::GetLimitOrder => {
                StromMessage::GetLimitOrders(RequestPair::<GetLimitOrders>::decode(buf)?)
            }
            StromMessageID::GetSearcherOrder => {
                StromMessage::GetSearcherOrders(RequestPair::<GetSearcherOrders>::decode(buf)?)
            }
        };
        Ok(ProtocolMessage { message_type, message })
    }
}

/// Encodes the protocol message into bytes.
/// The message type is encoded as a single byte and prepended to the message.
impl Encodable for ProtocolMessage {
    fn encode(&self, out: &mut dyn BufMut) {
        self.message_type.encode(out);
        self.message.encode(out);
    }

    fn length(&self) -> usize {
        self.message_type.length() + self.message.length()
    }
}

impl From<StromMessage> for ProtocolMessage {
    fn from(message: StromMessage) -> Self {
        ProtocolMessage { message_type: message.message_id(), message }
    }
}

/// Represents messages that can be sent to multiple peers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolBroadcastMessage {
    pub message_type: StromMessageID,
    pub message:      StromBroadcastMessage
}

/// Encodes the protocol message into bytes.
/// The message type is encoded as a single byte and prepended to the message.
impl Encodable for ProtocolBroadcastMessage {
    fn encode(&self, out: &mut dyn BufMut) {
        self.message_type.encode(out);
        self.message.encode(out);
    }

    fn length(&self) -> usize {
        self.message_type.length() + self.message.length()
    }
}

impl From<StromBroadcastMessage> for ProtocolBroadcastMessage {
    fn from(message: StromBroadcastMessage) -> Self {
        ProtocolBroadcastMessage { message_type: message.message_id(), message }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum StromMessage {
    /// init
    Status(Status),

    /// Consensus
    PrePropose(PreProposal),
    Proposal(Proposal),
    Commit(Commit),

    /// Propagation messages that broadcast new orders to all peers
    PropagateOrder(Vec<SignedLimitOrder),
    PropagateComposableOrder(SignedComposableLimitOrder),
    PropagateSearcherOrder(SignedSearcherOrder),
    PropagetComposableSearcherOrder(SignedComposableSearcherOrder),
    PropagateOrders(Vec<Orders>),

    // Order Request / Response pairs
    GetLimitOrders(RequestPair<GetLimitOrders>),
    LimitOrders(RequestPair<LimitOrders>),
    GetComposableLimitOrders(RequestPair<GetComposableLimitOrders>),
    ComposableLimitOrders(RequestPair<ComposableLimitOrders>),
    GetSearcherOrders(RequestPair<GetSearcherOrders>),
    SearcherOrders(RequestPair<SearcherOrders>),
    GetCompasableSearcherOrders(RequestPair<GetComposableSearcherOrders>),
    ComposableSearcherOrders(RequestPair<ComposableSearcherOrders>),
    GetAllOrders(RequestPair<GetOrders>),
    AllOrders(RequestPair<Orders>)
}

impl StromMessage {
    /// Returns the message's ID.
    pub fn message_id(&self) -> StromMessageID {
        match self {
            StromMessage::Status(_) => StromMessageID::Status,
            StromMessage::PropagateOrder(_) => StromMessageID::PropagateOrder,
            StromMessage::PropagateComposableOrder(_) => StromMessageID::PropagateComposableOrder,
            StromMessage::PropagateSearcherOrder(_) => StromMessageID::PropagateSearcherOrder,
            StromMessage::PropagetComposableSearcherOrder(_) => {
                StromMessageID::PropagetComposableSearcherOrder
            }
            StromMessage::PropagateOrders(_) => StromMessageID::PropagateOrders,
            StromMessage::GetLimitOrders(_) => StromMessageID::GetLimitOrders,
            StromMessage::LimitOrders(_) => StromMessageID::LimitOrders,
            StromMessage::GetComposableLimitOrders(_) => StromMessageID::GetComposableLimitOrders,
            StromMessage::ComposableLimitOrders(_) => StromMessageID::ComposableLimitOrders,
            StromMessage::GetSearcherOrders(_) => StromMessageID::GetSearcherOrders,
            StromMessage::SearcherOrders(_) => StromMessageID::SearcherOrders,
            StromMessage::GetCompasableSearcherOrders(_) => {
                StromMessageID::GetCompasableSearcherOrders
            }
            StromMessage::ComposableSearcherOrders(_) => StromMessageID::ComposableSearcherOrders,
            StromMessage::GetAllOrders(_) => StromMessageID::GetAllOrders,
            StromMessage::AllOrders(_) => StromMessageID::AllOrders,
            StromMessage::PrePropose(_) => StromMessageID::PrePropose,
            StromMessage::Proposal(_) => StromMessageID::Proposal,
            StromMessage::Commit(_) => StromMessageID::Commit
        }
    }
}

macro_rules! encodable_enum {
    ($enum_name:ident, $($var:ident),+) => {
        impl Encodable for $enum_name {
            fn encode(&self, out: &mut dyn BufMut) {
                match self {
                    $( $enum_name::$var(t) => t.encode(out), )*
                }
            }

            fn length(&self) -> usize {
                match self {
                    $( $enum_name::$var(t) => t.length(), )*
                }
            }
        }
    };
}

encodable_enum!(
    StromMessage,
    Status,
    PropagateOrder,
    PropagateComposableOrder,
    PropagateSearcherOrder,
    PropagetComposableSearcherOrder,
    PropagateOrders,
    GetLimitOrders,
    LimitOrders,
    GetComposableLimitOrders,
    ComposableLimitOrders,
    GetSearcherOrders,
    SearcherOrders,
    GetCompasableSearcherOrders,
    ComposableSearcherOrders,
    GetAllOrders,
    AllOrders,
    PrePropose,
    Proposal,
    Commit
);

/// Represents broadcast messages of [`StromMessage`] with the same object that
/// can be sent to multiple peers.
///
/// Messages that contain a list of hashes depend on the peer the message is
/// sent to. A peer should never receive a hash of an object (block,
/// transaction) it has already seen.
///
/// Note: This is only useful for outgoing messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StromBroadcastMessage {
    // Consensus Broadcast
    PrePropose(Arc<PreProposal>),
    Proposal(Arc<Proposal>),
    Commit(Arc<Commit>),

    PropagateComposableOrder(Arc<SignedComposableLimitOrder>),
    PropagateSearcherOrder(Arc<SignedSearcherOrder>),
    PropagetComposableSearcherOrder(Arc<SignedComposableSearcherOrder>),
    PropagateOrders(Arc<Vec<Orders>>)
}

// === impl StromBroadcastMessage ===

impl StromBroadcastMessage {
    /// Returns the message's ID.
    pub fn message_id(&self) -> StromMessageID {
        match self {
            StromBroadcastMessage::PropagateOrder(_) => StromMessageID::PropagateOrder,

            StromBroadcastMessage::PrePropose(_) => StromMessageID::PrePropose,
            StromBroadcastMessage::Proposal(_) => StromMessageID::Proposal,
            StromBroadcastMessage::Commit(_) => StromMessageID::Commit
        }
    }
}

encodable_enum!(StromBroadcastMessage, PropagateOrder, PrePropose, Proposal, Commit);

/// Represents message IDs for eth protocol messages.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum StromMessageID {
    Status           = 0,
    PropagateOrder   = 2,
    PrePropose       = 3,
    Proposal         = 4,
    Commit           = 5,
    UserOrder        = 6,
    SearcherOrder    = 7,
    LimitOrder       = 8,
    GetUserOrder     = 9,
    GetSearcherOrder = 10,
    GetLimitOrder    = 11
}

impl Encodable for StromMessageID {
    fn encode(&self, out: &mut dyn BufMut) {
        out.put_u8(*self as u8);
    }

    fn length(&self) -> usize {
        1
    }
}

impl Decodable for StromMessageID {
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        let id = buf.first().ok_or(alloy_rlp::Error::InputTooShort)?;
        let id = match id {
            0 => StromMessageID::Status,
            2 => StromMessageID::PropagateOrder,
            3 => StromMessageID::PrePropose,
            4 => StromMessageID::Proposal,
            5 => StromMessageID::Commit,
            _ => return Err(alloy_rlp::Error::Custom("Invalid message ID"))
        };
        buf.advance(1);
        Ok(id)
    }
}

impl TryFrom<usize> for StromMessageID {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(StromMessageID::Status),
            2 => Ok(StromMessageID::PropagateOrder),
            3 => Ok(StromMessageID::PrePropose),
            4 => Ok(StromMessageID::Proposal),
            5 => Ok(StromMessageID::Commit),
            _ => Err("Invalid message ID")
        }
    }
}

/// This is used for all request-response style `eth` protocol messages.
/// This can represent either a request or a response, since both include a
/// message payload and request id.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RequestPair<O> {
    /// id for the contained request or response message
    pub request_id: u64,

    /// the request or response message payload
    pub message: O
}

/// Allows messages with request ids to be serialized into RLP bytes.
impl<O> Encodable for RequestPair<O>
where
    O: Encodable
{
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        let header = Header {
            list:           true,
            payload_length: self.request_id.length() + self.message.length()
        };

        header.encode(out);
        self.request_id.encode(out);
        self.message.encode(out);
    }

    fn length(&self) -> usize {
        let mut length = 0;
        length += self.request_id.length();
        length += self.message.length();
        length += length_of_length(length);
        length
    }
}

/// Allows messages with request ids to be deserialized into RLP bytes.
impl<O> Decodable for RequestPair<O>
where
    O: Decodable
{
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        let _header = Header::decode(buf)?;
        Ok(Self { request_id: u64::decode(buf)?, message: O::decode(buf)? })
    }
}

#[cfg(test)]
mod test {
    // use hex_literal::hex;
    // use alloy_rlp::{Decodable, Encodable};
}
