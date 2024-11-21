use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration
};

use alloy::{
    primitives::{Address, BlockNumber, FixedBytes},
    providers::Provider,
    transports::Transport
};
use angstrom_metrics::ConsensusMetricsWrapper;
use angstrom_network::manager::StromConsensusEvent;
use angstrom_types::{
    consensus::{PreProposal, PreProposalAggregation, Proposal},
    contract_payloads::angstrom::{BundleGasDetails, UniswapAngstromRegistry},
    matching::uniswap::PoolSnapshot,
    orders::PoolSolution,
    primitive::PeerId,
    sol_bindings::grouped_orders::OrderWithStorageData
};
use bid_aggregation::BidAggregationState;
use futures::{future::BoxFuture, FutureExt, Stream};
use itertools::Itertools;
use matching_engine::MatchingEngineHandle;
use order_pool::order_storage::OrderStorage;
use uniswap_v4::uniswap::pool_manager::SyncedUniswapPools;

use crate::{AngstromValidator, Signer};

mod bid_aggregation;
mod finalization;
mod pre_proposal;
mod pre_proposal_aggregation;
mod proposal;

pub trait ConsensusState<T, Matching>: Send
where
    T: Transport + Clone,
    Matching: MatchingEngineHandle
{
    fn on_consensus_message(
        &mut self,
        handles: &mut Consensus<T, Matching>,
        message: StromConsensusEvent
    );

    /// just like streams. Once this returns Poll::Ready(None). This consensus
    /// round is over
    fn poll_transition(
        &mut self,
        handles: &mut Consensus<T, Matching>,
        cx: &mut Context<'_>
    ) -> Poll<Option<Box<dyn ConsensusState<T, Matching>>>>;
}

/// Holds and progresses the consensus state machine
pub struct RoundStateMachine<T, Matching> {
    current_state:           Box<dyn ConsensusState<T, Matching>>,
    /// for consensus, on a new block we wait a duration of time before signing
    /// our pre-proposal. this is the time
    consensus_wait_duration: Duration,
    consensus_arguments:     Consensus<T, Matching>
}

impl<T, Matching> RoundStateMachine<T, Matching>
where
    T: Transport + Clone,
    Matching: MatchingEngineHandle
{
    pub fn new(
        consensus_wait_duration: Duration,
        consensus_arguments: Consensus<T, Matching>
    ) -> Self {
        Self {
            current_state: Box::new(BidAggregationState::new(consensus_wait_duration)),
            consensus_wait_duration,
            consensus_arguments
        }
    }

    pub fn reset_round(&mut self, new_block: u64, new_leader: PeerId) {
        self.consensus_arguments.block_height = new_block;
        self.consensus_arguments.round_leader = new_leader;

        self.current_state = Box::new(BidAggregationState::new(self.consensus_wait_duration));
    }

    pub fn handle_message(&mut self, event: StromConsensusEvent) {
        self.current_state
            .on_consensus_message(&mut self.consensus_arguments, event);
    }
}

impl<T, Matching> Stream for RoundStateMachine<T, Matching>
where
    T: Transport + Clone,
    Matching: MatchingEngineHandle
{
    type Item = ConsensusTransitionMessage;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if let Poll::Ready(Some(transitioned_state)) = this
            .current_state
            .poll_transition(&mut this.consensus_arguments, cx)
        {
            this.current_state = transitioned_state;
        }

        if let Some(message) = this.consensus_arguments.messages.pop_front() {
            return Poll::Ready(Some(message))
        }

        Poll::Pending
    }
}

pub struct Consensus<T, Matching> {
    block_height:     BlockNumber,
    angstrom_address: Address,
    matching_engine:  Matching,
    signer:           Signer,
    round_leader:     PeerId,
    validators:       Vec<AngstromValidator>,
    order_storage:    Arc<OrderStorage>,
    _metrics:         ConsensusMetricsWrapper,
    pool_registry:    UniswapAngstromRegistry,
    uniswap_pools:    SyncedUniswapPools,
    provider:         Arc<Pin<Box<dyn Provider<T>>>>,
    messages:         VecDeque<ConsensusTransitionMessage>
}

// contains shared impls
impl<T, Matching> Consensus<T, Matching>
where
    T: Transport + Clone,
    Matching: MatchingEngineHandle
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_height: BlockNumber,
        angstrom_address: Address,
        order_storage: Arc<OrderStorage>,
        signer: Signer,
        round_leader: PeerId,
        validators: Vec<AngstromValidator>,
        metrics: ConsensusMetricsWrapper,
        pool_registry: UniswapAngstromRegistry,
        uniswap_pools: SyncedUniswapPools,
        provider: impl Provider<T> + 'static,
        matching_engine: Matching
    ) -> Self {
        Self {
            block_height,
            angstrom_address,
            round_leader,
            validators,
            order_storage,
            pool_registry,
            uniswap_pools,
            signer,
            _metrics: metrics,
            matching_engine,
            messages: VecDeque::new(),
            provider: Arc::new(Box::pin(provider))
        }
    }

    fn propagate_message(&mut self, message: ConsensusTransitionMessage) {
        self.messages.push_back(message);
    }

    fn i_am_leader(&self) -> bool {
        self.round_leader == self.signer.my_id
    }

    fn two_thirds_of_validation_set(&self) -> usize {
        (2 * self.validators.len()).div_ceil(3)
    }

    fn fetch_pool_snapshot(
        &self
    ) -> HashMap<FixedBytes<32>, (Address, Address, PoolSnapshot, u16)> {
        self.uniswap_pools
            .iter()
            .filter_map(|(key, pool)| {
                let (token_a, token_b, snapshot) =
                    pool.read().unwrap().fetch_pool_snapshot().ok()?;
                let entry = self.pool_registry.get_ang_entry(key)?;

                Some((*key, (token_a, token_b, snapshot, entry.store_index as u16)))
            })
            .collect::<HashMap<_, _>>()
    }

    fn matching_engine_output(
        &self,
        pre_proposal_aggregation: HashSet<PreProposalAggregation>
    ) -> BoxFuture<'static, eyre::Result<(Vec<PoolSolution>, BundleGasDetails)>> {
        // fetch
        let mut limit = Vec::new();
        let mut searcher = Vec::new();

        for pre_proposal_agg in pre_proposal_aggregation {
            pre_proposal_agg.pre_proposals.into_iter().for_each(|pre| {
                limit.extend(pre.limit);
                searcher.extend(pre.searcher);
            });
        }

        let limit = self.filter_quorum_orders(limit);
        let searcher = self.filter_quorum_orders(searcher);
        let pool_snapshots = self.fetch_pool_snapshot();

        let matcher = self.matching_engine.clone();

        async move { matcher.solve_pools(limit, searcher, pool_snapshots).await }.boxed()
    }

    fn filter_quorum_orders<O: Hash + Eq + Clone>(
        &self,
        input: Vec<OrderWithStorageData<O>>
    ) -> Vec<OrderWithStorageData<O>> {
        let two_thirds = self.two_thirds_of_validation_set();
        input
            .into_iter()
            .fold(HashMap::new(), |mut acc, order| {
                *acc.entry(order).or_insert(0) += 1;
                acc
            })
            .into_iter()
            .filter(|(_, count)| *count >= two_thirds)
            .map(|(order, _)| order)
            .collect()
    }

    fn handle_pre_proposal_aggregation(
        &mut self,
        peer_id: PeerId,
        pre_proposal_agg: PreProposalAggregation,
        pre_proposal_agg_set: &mut HashSet<PreProposalAggregation>
    ) {
        self.handle_proposal_verification(
            peer_id,
            pre_proposal_agg,
            pre_proposal_agg_set,
            |proposal, block| proposal.is_valid(block)
        )
    }

    fn verify_proposal(&mut self, peer_id: PeerId, proposal: Proposal) -> Option<Proposal> {
        if self.round_leader != peer_id {
            return None
        }

        proposal.is_valid(&self.block_height).then(|| {
            self.messages
                .push_back(ConsensusTransitionMessage::PropagateProposal(proposal.clone()));

            proposal
        })
    }

    fn handle_pre_proposal(
        &mut self,
        peer_id: PeerId,
        pre_proposal: PreProposal,
        pre_proposal_set: &mut HashSet<PreProposal>
    ) {
        self.handle_proposal_verification(
            peer_id,
            pre_proposal,
            pre_proposal_set,
            |proposal, block| proposal.is_valid(block)
        )
    }

    fn handle_proposal_verification<P>(
        &mut self,
        peer_id: PeerId,
        proposal: P,
        proposal_set: &mut HashSet<P>,
        valid: impl FnOnce(&P, &BlockNumber) -> bool
    ) where
        P: Into<ConsensusTransitionMessage> + Eq + Hash + Clone
    {
        if !self.validators.iter().map(|v| v.peer_id).contains(&peer_id) {
            tracing::warn!(peer=?peer_id,"got a consensus message from a invalid peer");
            return
        }
        // ensure pre_proposal is valid
        if !valid(&proposal, &self.block_height) {
            tracing::info!(peer=?peer_id,"got a invalid consensus message");
            return
        }

        // if  we don't have the pre_proposal, propagate it and then store it.
        // else log a message
        if !proposal_set.contains(&proposal) {
            self.propagate_message(proposal.clone().into());
            proposal_set.insert(proposal);
        } else {
            tracing::info!(peer=?peer_id,"got a duplicate consensus message");
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConsensusTransitionMessage {
    /// Either our or another nodes PreProposal. The PreProposal will only be
    /// shared here if it is new to this nodes consensus outlook
    PropagatePreProposal(PreProposal),
    PropagatePreProposalAgg(PreProposalAggregation),
    PropagateProposal(Proposal)
}

impl From<PreProposal> for ConsensusTransitionMessage {
    fn from(value: PreProposal) -> Self {
        Self::PropagatePreProposal(value)
    }
}

impl From<PreProposalAggregation> for ConsensusTransitionMessage {
    fn from(value: PreProposalAggregation) -> Self {
        Self::PropagatePreProposalAgg(value)
    }
}
