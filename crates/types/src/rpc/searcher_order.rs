use alloy_primitives::{Address, U256};
use alloy_rlp::{Decodable, Encodable, Error};
use alloy_rlp_derive::{RlpDecodable, RlpEncodable};
use alloy_sol_types::SolStruct;
use derive_more::{AsRef, Deref};
use reth_primitives::{recover_signer, Signature as ESignature};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::primitive::{Angstrom::Order, Signature, ANGSTROM_DOMAIN};

/// Submitted order pre-processing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable)]
pub struct SignedSearcherOrder {
    /// The original order from the user.
    pub details:   Order,
    /// The user's EIP-712 signature of the Order.
    pub signature: Signature
}

//TODO: Also implement recovery for 1271 orders see
impl SignedSearcherOrder {
    pub fn recover_signer(&self) -> Option<Address> {
        let hash = self.details.eip712_signing_hash(&ANGSTROM_DOMAIN);
        Some(self.signature.0.recover_signer(hash))?
    }
}

/// Signed transaction with recovered signer.
#[derive(Debug, Clone, PartialEq, Hash, Eq, AsRef, Deref)]
pub struct EcRecoveredSearcherOrder {
    /// Signer of the transaction
    signer:             Address,
    /// Signed transaction
    #[deref]
    #[as_ref]
    signed_transaction: SignedSearcherOrder
}
