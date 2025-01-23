use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher}
};

use alloy::{
    primitives::{keccak256, BlockNumber},
    signers::{Signature, SignerSync}
};
use alloy_primitives::U256;
use bytes::Bytes;
use reth_network_peers::PeerId;
use serde::{Deserialize, Serialize};

use crate::{
    orders::OrderSet,
    primitive::{AngstromSigner, PoolId},
    sol_bindings::{
        grouped_orders::{GroupedVanillaOrder, OrderWithStorageData},
        rpc_orders::TopOfBlockOrder
    }
};

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SearcherOrder {
    pub tobo:       OrderWithStorageData<TopOfBlockOrder>,
    pub tob_reward: U256
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreProposal {
    pub block_height: BlockNumber,
    pub source:       PeerId,
    pub limit:        HashMap<PoolId, OrderWithStorageData<GroupedVanillaOrder>>,
    pub searcher:     HashMap<PoolId, SearcherOrder>,
    pub signature:    Signature
}

impl Hash for PreProposal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the block height
        self.block_height.hash(state);

        // Hash the source
        self.source.hash(state);

        // Hash the limit HashMap in a deterministic way
        let mut limit_keys: Vec<_> = self.limit.keys().collect();
        limit_keys.sort(); // Ensure consistent order
        for key in limit_keys {
            key.hash(state);
            self.limit[key].hash(state);
        }

        // Hash the searcher HashMap in a deterministic way
        let mut searcher_keys: Vec<_> = self.searcher.keys().collect();
        searcher_keys.sort(); // Ensure consistent order
        for key in searcher_keys {
            key.hash(state);
            self.searcher[key].hash(state);
        }

        // Hash the signature
        self.signature.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreProposalContent {
    pub block_height: BlockNumber,
    pub source:       PeerId,
    pub limit:        HashMap<PoolId, OrderWithStorageData<GroupedVanillaOrder>>,
    pub searcher:     HashMap<PoolId, SearcherOrder>
}

// the reason for the manual implementation is because EcDSA signatures are not
// deterministic. EdDSA ones are, but the below allows for one less footgun
// If the struct switches to BLS, or any type of multisig or threshold
// signature, then the implementation should be changed to include it
impl std::hash::Hash for PreProposalContent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.block_height.hash(state);
        self.source.hash(state);

        // Sort and hash the limit orders
        let mut limit_keys: Vec<_> = self.limit.keys().collect();
        limit_keys.sort();
        for key in limit_keys {
            key.hash(state);
            self.limit[key].hash(state);
        }

        // Sort and hash the searcher orders
        let mut searcher_keys: Vec<_> = self.searcher.keys().collect();
        searcher_keys.sort();
        for key in searcher_keys {
            key.hash(state);
            self.searcher[key].hash(state);
        }
    }
}
impl PreProposal {
    pub fn content(&self) -> PreProposalContent {
        PreProposalContent {
            block_height: self.block_height,
            source:       self.source,
            limit:        self.limit.clone(),
            searcher:     self.searcher.clone()
        }
    }
}

impl PreProposal {
    fn sign_payload(sk: &AngstromSigner, payload: Vec<u8>) -> Signature {
        let hash = keccak256(payload);
        sk.sign_hash_sync(&hash).unwrap()
    }

    pub fn generate_pre_proposal(
        ethereum_height: BlockNumber,
        sk: &AngstromSigner,
        limit: HashMap<PoolId, OrderWithStorageData<GroupedVanillaOrder>>,
        searcher: HashMap<PoolId, SearcherOrder>
    ) -> Self {
        let payload = Self::serialize_payload(&ethereum_height, &limit, &searcher);
        let signature = Self::sign_payload(sk, payload);

        Self { limit, source: sk.id(), searcher, block_height: ethereum_height, signature }
    }

    pub fn new(
        ethereum_height: u64,
        sk: &AngstromSigner,
        orders: OrderSet<GroupedVanillaOrder, TopOfBlockOrder>
    ) -> Self {
        let OrderSet { limit, searcher } = orders;

        let limit_map: HashMap<PoolId, OrderWithStorageData<GroupedVanillaOrder>> = limit
            .into_iter()
            .map(|order_with_storage| {
                let pool_id = order_with_storage.pool_id;
                (pool_id, order_with_storage.clone())
            })
            .collect();
        let searcher_map: HashMap<PoolId, SearcherOrder> = searcher
            .into_iter()
            .map(|order_with_storage| {
                let pool_id = order_with_storage.pool_id;
                let searcher_order = SearcherOrder {
                    tobo:       order_with_storage.clone(), // maybe shouldnt clone here
                    tob_reward: order_with_storage.tob_reward
                };
                (pool_id, searcher_order)
            })
            .collect();

        // Call generate_pre_proposal with HashMap
        Self::generate_pre_proposal(ethereum_height, sk, limit_map, searcher_map)
    }

    /// ensures block height is correct as-well as validates the signature.
    pub fn is_valid(&self, block_height: &BlockNumber) -> bool {
        let hash = keccak256(self.payload());
        let Ok(source) = self.signature.recover_from_prehash(&hash) else {
            return false;
        };
        let source = AngstromSigner::public_key_to_peer_id(&source);

        source == self.source && &self.block_height == block_height
    }

    fn serialize_payload(
        block_height: &BlockNumber,
        limit: &HashMap<PoolId, OrderWithStorageData<GroupedVanillaOrder>>,
        searcher: &HashMap<PoolId, SearcherOrder>
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend(bincode::serialize(block_height).unwrap());

        // Serialize limit orders in sorted order
        let mut limit_keys: Vec<_> = limit.keys().collect();
        limit_keys.sort();
        for key in limit_keys {
            buf.extend(bincode::serialize(key).unwrap());
            buf.extend(bincode::serialize(&limit[key]).unwrap());
        }

        // Serialize searcher orders in sorted order
        let mut searcher_keys: Vec<_> = searcher.keys().collect();
        searcher_keys.sort();
        for key in searcher_keys {
            buf.extend(bincode::serialize(key).unwrap());
            buf.extend(bincode::serialize(&searcher[key]).unwrap());
        }

        buf
    }

    fn payload(&self) -> Bytes {
        Bytes::from(Self::serialize_payload(&self.block_height, &self.limit, &self.searcher))
    }

    pub fn orders_by_pool_id(
        preproposals: &[PreProposal]
    ) -> HashMap<PoolId, HashSet<OrderWithStorageData<GroupedVanillaOrder>>> {
        preproposals.iter().flat_map(|p| p.limit.iter()).fold(
            HashMap::new(),
            |mut acc, (pool_id, order_with_storage)| {
                acc.entry(*pool_id)
                    .or_default()
                    .insert(order_with_storage.clone());
                acc
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::PreProposal;
    use crate::primitive::AngstromSigner;

    #[test]
    fn can_be_constructed() {
        let ethereum_height = 100;
        let limit = HashMap::new();
        let searcher = HashMap::new();
        let sk = AngstromSigner::random();
        PreProposal::generate_pre_proposal(ethereum_height, &sk, limit, searcher);
    }

    #[test]
    fn can_validate_self() {
        let ethereum_height = 100;
        let limit = HashMap::new();
        let searcher = HashMap::new();
        let sk = AngstromSigner::random();
        let preproposal = PreProposal::generate_pre_proposal(ethereum_height, &sk, limit, searcher);

        assert!(preproposal.is_valid(&ethereum_height), "Unable to validate self");
    }
}
