use bincode::{config::standard, encode_to_vec, Decode, Encode};
use bytes::Bytes;
use reth_network_peers::PeerId;
use reth_primitives::keccak256;
use secp256k1::SecretKey;

use super::PreProposal;
use crate::{orders::PoolSolution, primitive::Signature};

#[derive(Default, Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Proposal {
    // Might not be necessary as this is encoded in all the proposals anyways
    pub ethereum_height: u64,
    #[bincode(with_serde)]
    pub source:          PeerId,
    pub preproposals:    Vec<PreProposal>,
    pub solutions:       Vec<PoolSolution>,
    /// This signature is over (etheruem_block | hash(vanilla_bundle) |
    /// hash(order_buffer) | hash(lower_bound))
    pub signature:       Signature
}

impl Proposal {
    pub fn generate_proposal(
        ethereum_height: u64,
        source: PeerId,
        preproposals: Vec<PreProposal>,
        solutions: Vec<PoolSolution>,
        sk: &SecretKey
    ) -> Self {
        let mut buf = Vec::new();
        let std = standard();
        buf.extend(encode_to_vec(ethereum_height, std).unwrap());
        buf.extend(*source);
        buf.extend(encode_to_vec(&preproposals, std).unwrap());
        buf.extend(encode_to_vec(&solutions, std).unwrap());

        let hash = keccak256(buf);
        let sig = reth_primitives::sign_message(sk.secret_bytes().into(), hash).unwrap();

        Self { ethereum_height, source, preproposals, solutions, signature: Signature(sig) }
    }

    pub fn validate(&self) -> bool {
        let hash = keccak256(self.payload());
        let Ok(source) = self.signature.recover_signer_full_public_key(hash) else {
            return false;
        };
        source == self.source
    }

    fn payload(&self) -> Bytes {
        let mut buf = Vec::new();
        let std = standard();
        buf.extend(encode_to_vec(self.ethereum_height, std).unwrap());
        buf.extend(*self.source);
        buf.extend(encode_to_vec(&self.preproposals, std).unwrap());
        buf.extend(encode_to_vec(&self.solutions, std).unwrap());

        Bytes::from_iter(buf)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::FixedBytes;
    use rand::thread_rng;
    use reth_network_peers::pk2id;
    use secp256k1::Secp256k1;

    use super::{Proposal, SecretKey};

    #[test]
    fn can_be_constructed() {
        let ethereum_height = 100;
        let source = FixedBytes::random();
        let preproposals = vec![];
        let solutions = vec![];
        let mut rng = thread_rng();
        let sk = SecretKey::new(&mut rng);
        Proposal::generate_proposal(ethereum_height, source, preproposals, solutions, &sk);
    }

    #[test]
    fn can_validate_self() {
        let ethereum_height = 100;
        let preproposals = vec![];
        let solutions = vec![];
        // Generate crypto stuff
        let mut rng = thread_rng();
        let sk = SecretKey::new(&mut rng);
        let secp = Secp256k1::new();
        let pk = sk.public_key(&secp);
        // Grab the source ID from the secret/public keypair
        let source = pk2id(&pk);
        let proposal =
            Proposal::generate_proposal(ethereum_height, source, preproposals, solutions, &sk);

        assert!(proposal.validate(), "Unable to validate self");
    }
}
