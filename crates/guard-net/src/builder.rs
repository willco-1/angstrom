//! Builder structs for messages.

use reth_primitives::{Chain, ForkId, B256, U256};

use crate::Status;

/// Builder for [`Status`] messages.
#[derive(Debug, Default)]
pub struct StatusBuilder {
    status: Status
}

impl StatusBuilder {
    /// Consumes the type and creates the actual [`Status`] message.
    pub fn build(self) -> Status {
        self.status
    }

    /// Sets the protocol version.
    pub fn version(mut self, version: u8) -> Self {
        self.status.version = version;
        self
    }

    /// Sets the chain id.
    pub fn chain(mut self, chain: Chain) -> Self {
        self.status.chain = chain;
        self
    }

    /// Sets the total difficulty.
    pub fn total_difficulty(mut self, total_difficulty: U256) -> Self {
        self.status.total_difficulty = total_difficulty;
        self
    }

    /// Sets the block hash.
    pub fn blockhash(mut self, blockhash: B256) -> Self {
        self.status.blockhash = blockhash;
        self
    }

    /// Sets the genesis hash.
    pub fn genesis(mut self, genesis: B256) -> Self {
        self.status.genesis = genesis;
        self
    }

    /// Sets the fork id.
    pub fn forkid(mut self, forkid: ForkId) -> Self {
        self.status.forkid = forkid;
        self
    }
}
