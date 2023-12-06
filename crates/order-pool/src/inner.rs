use std::{
    collections::HashMap,
    marker::Unpin,
    pin::Pin,
    task::{Context, Poll}
};

use alloy_primitives::B256;
use futures_util::{Stream, StreamExt};
use guard_types::{
    orders::{
        OrderId, OrderLocation, OrderOrigin, OrderPriorityData, PooledComposableOrder,
        PooledLimitOrder, PooledOrder, PooledSearcherOrder, SearcherPriorityData, ValidatedOrder,
        ValidationResults
    },
    primitive::PoolId
};
use reth_primitives::Address;
use validation::order::OrderValidator;

use crate::{
    common::FilledOrder, config::PoolConfig, finalization_pool::FinalizationPool,
    limit::LimitOrderPool, searcher::SearcherPool, validator::Validator, BidsAndAsks, OrderSet
};

pub struct OrderPoolInner<L, CL, S, CS, V>
where
    L: PooledLimitOrder,
    CL: PooledComposableOrder + PooledLimitOrder,
    S: PooledSearcherOrder,
    CS: PooledComposableOrder + PooledSearcherOrder,
    V: OrderValidator
{
    limit_pool:        LimitOrderPool<L, CL>,
    searcher_pool:     SearcherPool<S, CS>,
    finalization_pool: FinalizationPool<L, CL, S, CS>,
    _config:           PoolConfig,
    /// Address to order id, used for nonce lookups
    address_to_orders: HashMap<Address, Vec<OrderId>>,
    /// Order hash to order id, used for order inclusion lookups
    hash_to_order_id:  HashMap<B256, OrderId>,
    /// Order Validator
    validator:         Validator<L, CL, S, CS, V>
}

impl<L, CL, S, CS, V> OrderPoolInner<L, CL, S, CS, V>
where
    L: PooledLimitOrder<ValidationData = OrderPriorityData>,
    CL: PooledComposableOrder + PooledLimitOrder<ValidationData = OrderPriorityData>,

    S: PooledSearcherOrder<ValidationData = SearcherPriorityData>,
    CS: PooledComposableOrder + PooledSearcherOrder<ValidationData = SearcherPriorityData>,
    V: OrderValidator<
        LimitOrder = L,
        SearcherOrder = S,
        ComposableLimitOrder = CL,
        ComposableSearcherOrder = CS
    >
{
    pub fn new(validator: V, config: PoolConfig) -> Self {
        Self {
            limit_pool:        LimitOrderPool::new(None),
            searcher_pool:     SearcherPool::new(None),
            finalization_pool: FinalizationPool::new(),
            _config:           config,
            address_to_orders: HashMap::new(),
            hash_to_order_id:  HashMap::new(),
            validator:         Validator::new(validator)
        }
    }

    pub fn new_limit_order(&mut self, origin: OrderOrigin, order: L) {
        self.validator.validate_order(origin, order);
    }

    pub fn new_composable_limit(&mut self, origin: OrderOrigin, order: CL) {
        self.validator.validate_composable_order(origin, order);
    }

    pub fn new_searcher_order(&mut self, origin: OrderOrigin, order: S) {
        self.validator.validate_searcher_order(origin, order)
    }

    pub fn new_composable_searcher_order(&mut self, origin: OrderOrigin, order: CS) {
        self.validator
            .validate_composable_searcher_order(origin, order)
    }

    pub fn fetch_vanilla_orders(&self) -> OrderSet<L, S> {
        let limit = self.limit_pool.fetch_all_vanilla_orders();
        let searcher = self.searcher_pool.get_winning_orders_vanilla();

        OrderSet { limit, searcher }
    }

    pub fn fetch_composable_orders(&self) -> OrderSet<CL, CS> {
        let limit = self.limit_pool.fetch_all_composable_orders();
        let searcher = self.searcher_pool.get_winning_orders_composable();

        OrderSet { limit, searcher }
    }

    pub fn eoa_state_change(&mut self, eoas: Vec<Address>) {
        eoas.into_iter()
            .filter_map(|eoa| self.address_to_orders.remove(&eoa))
            .for_each(|order_ids| {
                order_ids.into_iter().for_each(|id| match id.location {
                    OrderLocation::Composable => {
                        if let Some(order) = self.limit_pool.remove_composable_limit_order(&id.hash)
                        {
                            self.validator
                                .validate_composable_order(OrderOrigin::Local, order.order);
                        }
                    }
                    OrderLocation::LimitParked | OrderLocation::LimitPending => {
                        if let Some(order) =
                            self.limit_pool.remove_limit_order(&id.hash, id.location)
                        {
                            self.validator
                                .validate_order(OrderOrigin::Local, order.order);
                        }
                    }

                    OrderLocation::VanillaSearcher => {
                        if let Ok(order) = self.searcher_pool.remove_searcher_order(id) {
                            self.validator
                                .validate_searcher_order(OrderOrigin::Local, order.order);
                        }
                    }
                    OrderLocation::ComposableSearcher => {
                        if let Ok(order) = self.searcher_pool.remove_composable_searcher_order(id) {
                            self.validator
                                .validate_composable_searcher_order(OrderOrigin::Local, order.order)
                        }
                    }
                })
            });
    }

    pub fn finalized_block(&mut self, block: u64) -> Vec<FilledOrder<L, CL, S, CS>> {
        self.finalization_pool.finalized(block)
    }

    pub fn reorg(&mut self, orders: Vec<B256>) {
        self.finalization_pool
            .reorg(orders)
            .for_each(|order| match order {
                FilledOrder::ComposableSearcher(cs) => self
                    .validator
                    .validate_composable_searcher_order(OrderOrigin::Local, cs),
                FilledOrder::ComposableLimit(cl) => self
                    .validator
                    .validate_composable_order(OrderOrigin::Local, cl),
                FilledOrder::Limit(l) => self.validator.validate_order(OrderOrigin::Local, l),
                FilledOrder::Searcher(s) => self
                    .validator
                    .validate_searcher_order(OrderOrigin::Local, s)
            });
    }

    /// Removes all filled orders from the pools
    pub fn filled_orders(&mut self, block: u64, orders: &Vec<B256>) {
        // remove from lower level + hash locations;
        self.finalization_pool.new_orders(
            block,
            orders.iter().filter_map(|order_hash| {
                let order_id = self.hash_to_order_id.remove(order_hash)?;
                let loc = order_id.location;
                match loc {
                    OrderLocation::Composable => self
                        .limit_pool
                        .remove_composable_limit_order(order_hash)
                        .map(|o| o.order)
                        .map(FilledOrder::add_composable_limit),
                    OrderLocation::LimitParked | OrderLocation::LimitPending => self
                        .limit_pool
                        .remove_limit_order(order_hash, loc)
                        .map(|o| o.order)
                        .map(FilledOrder::add_limit),
                    OrderLocation::VanillaSearcher => self
                        .searcher_pool
                        .remove_searcher_order(order_id)
                        .inspect_err(|e| eprint!("{e:?}"))
                        .ok()
                        .map(|o| o.order)
                        .map(FilledOrder::add_searcher),
                    OrderLocation::ComposableSearcher => self
                        .searcher_pool
                        .remove_composable_searcher_order(order_id)
                        .inspect_err(|e| eprint!("{e:?}"))
                        .ok()
                        .map(|o| o.order)
                        .map(FilledOrder::add_composable_searcher)
                }
            })
        )
    }
}

impl<L, CL, S, CS, V> OrderPoolInner<L, CL, S, CS, V>
where
    L: PooledLimitOrder,
    CL: PooledComposableOrder + PooledLimitOrder,
    S: PooledSearcherOrder,
    CS: PooledComposableOrder + PooledSearcherOrder,
    V: OrderValidator
{
    fn handle_validated_order(
        &mut self,
        _res: ValidationResults<L, CL, S, CS>
    ) -> Option<OrdersToPropagate<L, CL, S, CS>> {
        todo!()
    }
}

impl<L, CL, S, CS, V> Stream for OrderPoolInner<L, CL, S, CS, V>
where
    L: PooledLimitOrder,
    CL: PooledComposableOrder + PooledLimitOrder,
    S: PooledSearcherOrder,
    CS: PooledComposableOrder + PooledSearcherOrder,
    V: OrderValidator + Unpin
{
    type Item = OrdersToPropagate<L, CL, S, CS>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(Some(next)) = self.validator.poll_next_unpin(cx) {
            if let Some(prop) = self.handle_validated_order(next) {
                return Poll::Ready(Some(prop))
            }
        }

        Poll::Pending
    }
}

pub enum OrdersToPropagate<L, CL, S, CS> {
    Limit(L),
    LimitComposable(CL),
    Searcher(S),
    SearcherCompsable(CS)
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum PoolError {
    #[error("Pool has reached max size, and order doesn't satisify replacment requirements")]
    MaxSize,
    #[error("No pool was found for address: {0}")]
    NoPool(PoolId),
    #[error("Already have a ordered with {0:?}")]
    DuplicateNonce(OrderId),
    #[error("Duplicate order")]
    DuplicateOrder
}
