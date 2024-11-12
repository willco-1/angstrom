mod common;
mod config;
mod finalization_pool;
mod limit;
mod order_indexer;
pub mod order_storage;

mod searcher;
mod validator;

use std::future::Future;

use alloy::primitives::{Address, FixedBytes, B256};
use angstrom_types::{
    orders::{OrderLocation, OrderOrigin, OrderStatus},
    sol_bindings::grouped_orders::AllOrders
};
pub use angstrom_utils::*;
pub use config::PoolConfig;
pub use order_indexer::*;
use tokio::sync::broadcast::Receiver;

#[derive(Debug, Clone)]
pub enum PoolManagerUpdate {
    NewOrder(AllOrders),
    FilledOrder((u64, AllOrders)),
    UnfilledOrders(AllOrders),
    CancelledOrder(B256)
}

/// The OrderPool Trait is how other processes can interact with the orderpool
/// asyncly. This allows for requesting data and providing data from different
/// threads efficiently.
pub trait OrderPoolHandle: Send + Sync + Clone + Unpin + 'static {
    fn new_order(&self, origin: OrderOrigin, order: AllOrders)
        -> impl Future<Output = bool> + Send;

    fn subscribe_orders(&self) -> Receiver<PoolManagerUpdate>;

    fn pending_orders(&self, sender: Address) -> impl Future<Output = Vec<AllOrders>> + Send;

    fn cancel_order(&self, sender: Address, order_hash: B256) -> impl Future<Output = bool> + Send;

    fn fetch_orders_from_pool(
        &self,
        pool_id: FixedBytes<32>,
        location: OrderLocation
    ) -> impl Future<Output = Vec<AllOrders>> + Send;

    fn fetch_order_status(
        &self,
        order_hash: B256
    ) -> impl Future<Output = Option<OrderStatus>> + Send;
}
