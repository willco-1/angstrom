use std::collections::HashMap;

use reth_primitives::B256;

use super::{LimitOrderLocation, LimitPoolError, PoolId};
use crate::{
    common::{ParkedPool, PendingPool},
    PooledLimitOrder
};

pub struct LimitPool<T: PooledLimitOrder> {
    pending_orders: HashMap<PoolId, PendingPool<T>>,
    parked_orders:  HashMap<PoolId, ParkedPool<T>>
}

impl<T: PooledLimitOrder> LimitPool<T> {
    pub fn new() -> Self {
        todo!()
    }

    pub fn new_order(&mut self, order: T) -> Result<LimitOrderLocation, LimitPoolError> {
        let pool_addr = order.get_pool();

        if order.is_valid() {
            self.pending_orders
                .get_mut(&pool_addr)
                .map(|pool| pool.new_order(order))
                .ok_or_else(|| LimitPoolError::NoPool(pool_addr))??;
            Ok(LimitOrderLocation::LimitPending)
        } else {
            self.parked_orders
                .get_mut(&pool_addr)
                .map(|pool| pool.new_order(order))
                .ok_or_else(|| LimitPoolError::NoPool(pool_addr))??;
            Ok(LimitOrderLocation::LimitParked)
        }
    }

    pub fn filled_orders(&mut self, orders: &Vec<B256>) -> Vec<T> {
        vec![]
    }
}
