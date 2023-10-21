use std::{
    pin::Pin,
    task::{Context, Poll}
};

use common::{ConsensusState, WAITING_NEXT_BLOCK};
use futures::FutureExt;
use guard_types::on_chain::BestSolvedBundleData;

use super::{
    completed::CompletedState, GlobalStateContext, RoundAction, RoundStateMessage, StateTransition,
    Timeout
};

/// This state is only reached if this guard is the leader
pub struct SubmitState {
    submit_deadline: Timeout,
    best_bundle:     BestSolvedBundleData,
    current_commits: Vec<()>,
    needed_commits:  usize,
    can_send:        bool
}

impl SubmitState {
    pub fn new() -> Self {
        todo!()
    }

    pub fn on_new_commit(&mut self, commit: ()) {
        // check if contains if
        if self.current_commits.len() == self.needed_commits {
            self.can_send = true;
        }
    }
}

impl StateTransition for SubmitState {
    fn should_transition(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _: GlobalStateContext
    ) -> Poll<(RoundAction, ConsensusState, Option<RoundStateMessage>)> {
        self.submit_deadline.poll_unpin(cx).map(|_| {
            if self.can_send {
                // submission here
                (
                    RoundAction::Completed(CompletedState),
                    WAITING_NEXT_BLOCK,
                    Some(RoundStateMessage::RelaySubmission())
                )
            } else {
                (RoundAction::Completed(CompletedState), WAITING_NEXT_BLOCK, None)
            }
        })
    }
}
