use crate::{
    contract_bindings::Angstrom::PoolKey,
    on_chain::{Signature, SubmittedOrder}
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolOrders {
    pub pool:         PoolKey,
    pub searcher_bid: SubmittedOrder,
    pub sorted_bids:  Vec<SubmittedOrder>,
    pub sorted_asks:  Vec<SubmittedOrder>
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreProposal {
    pub ethereum_height: u64,
    pub pre_bundle:      Vec<PoolOrders>,
    /// the signature is over the ethereum height and the bundle hash
    /// sign(ethereum_height | hash(pre_bundle))
    pub signature:       Signature
}