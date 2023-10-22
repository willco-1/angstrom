use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll}
};

use action::RelaySender;
use common::{return_if, PollExt};
use ethers_core::types::{Block, H256};
use ethers_flashbots::PendingBundleError;
use ethers_providers::{Middleware, PubsubClient, RpcError, SubscriptionStream};
use futures::Stream;
use futures_util::StreamExt;
use guard_network::{Swarm, SwarmEvent};
use guard_types::on_chain::{SubmissionBundle, SubmittedOrder, VanillaBundle};

use crate::submission_server::{Submission, SubmissionServer};

pub enum NetworkManagerMsg {
    Swarm(SwarmEvent),
    SubmissionServer(Submission),
    NewEthereumBlock(Block<H256>),
    RelaySubmission(Result<(), PendingBundleError>)
}

/// Holds all of our network state
pub struct NetworkManager<M: Middleware + 'static>
where
    <M as Middleware>::Provider: PubsubClient
{
    /// guard network connection
    guard_net:         Swarm,
    /// deals with new submissions through a rpc to the network
    submission_server: SubmissionServer,
    /// for the leader to submit to relays
    relay_sender:      RelaySender<M>,
    /// general new block stream. Will be updated when our local optimized
    /// mem-pool is built
    block_stream:      SubscriptionStream<'static, M::Provider, Block<H256>>
}

impl<M: Middleware + 'static> NetworkManager<M>
where
    <M as Middleware>::Provider: PubsubClient
{
    pub async fn new(
        middleware: &'static M,
        guard_net: Swarm,
        submission_server: SubmissionServer,
        relay_sender: RelaySender<M>
    ) -> anyhow::Result<Self> {
        let block_stream = middleware.subscribe_blocks().await?;

        Ok(Self { relay_sender, guard_net, submission_server, block_stream })
    }

    /// grabs the guard network handle
    pub fn guard_net_mut(&mut self) -> &mut Swarm {
        &mut self.guard_net
    }

    /// sends the bundle to all specified relays
    pub fn send_to_relay(&mut self, bundle: SubmissionBundle) {
        self.relay_sender.submit_bundle(bundle);
    }

    /// used to share new txes with externally subscribed users
    pub fn on_new_user_txes(&mut self, tx: Arc<SubmittedOrder>) {
        self.submission_server.on_new_user_tx(tx);
    }

    /// used to share new bundles with externally subscribed users
    pub fn on_new_best_bundle(&mut self, bundle: Arc<VanillaBundle>) {
        self.submission_server.on_new_best_bundle(bundle)
    }
}

impl<M: Middleware + 'static> Stream for NetworkManager<M>
where
    <M as Middleware>::Provider: PubsubClient
{
    type Item = NetworkManagerMsg;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        return_if!(
        self
            .guard_net
            .poll_next_unpin(cx)
            .filter_map(|poll| poll)
            .map(|event| Some(NetworkManagerMsg::Swarm(event))) => { is_ready() }
        );

        return_if!(
        self
            .submission_server
            .poll_next_unpin(cx)
            .filter_map(|poll| poll)
            .map(|event| Some(NetworkManagerMsg::SubmissionServer(event))) => { is_ready() }
        );

        return_if!(
        self
            .block_stream
            .poll_next_unpin(cx)
            .filter_map(|poll| poll)
            .map(|event| Some(NetworkManagerMsg::NewEthereumBlock(event))) =>{ is_ready() }
        );

        return_if!(
        self
            .relay_sender
            .poll(cx)
            .map(|event| Some(NetworkManagerMsg::RelaySubmission(event))) => { is_ready() }
        );

        Poll::Pending
    }
}