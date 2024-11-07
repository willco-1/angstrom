//! basic book impl so we can benchmark
use angstrom_types::{
    matching::uniswap::PoolSnapshot,
    primitive::PoolId,
    sol_bindings::grouped_orders::{GroupedVanillaOrder, OrderWithStorageData}
};

use self::sort::SortStrategy;

pub mod order;
pub mod sort;

#[derive(Debug, Default)]
pub struct OrderBook {
    id:   PoolId,
    amm:  Option<PoolSnapshot>,
    bids: Vec<OrderWithStorageData<GroupedVanillaOrder>>,
    asks: Vec<OrderWithStorageData<GroupedVanillaOrder>>
}

impl OrderBook {
    pub fn new(
        id: PoolId,
        amm: Option<PoolSnapshot>,
        mut bids: Vec<OrderWithStorageData<GroupedVanillaOrder>>,
        mut asks: Vec<OrderWithStorageData<GroupedVanillaOrder>>,
        sort: Option<SortStrategy>
    ) -> Self {
        // Use our sorting strategy to sort our bids and asks
        let strategy = sort.unwrap_or_default();
        strategy.sort_bids(&mut bids);
        strategy.sort_asks(&mut asks);
        Self { id, amm, bids, asks }
    }

    pub fn id(&self) -> PoolId {
        self.id
    }

    pub fn bids(&self) -> &Vec<OrderWithStorageData<GroupedVanillaOrder>> {
        &self.bids
    }

    pub fn asks(&self) -> &Vec<OrderWithStorageData<GroupedVanillaOrder>> {
        &self.asks
    }

    pub fn amm(&self) -> Option<&PoolSnapshot> {
        self.amm.as_ref()
    }
}

#[cfg(test)]
mod test {
    use alloy::primitives::FixedBytes;
    use angstrom_types::matching::SqrtPriceX96;

    use super::*;

    #[test]
    fn can_construct_order_book() {
        // Very basic book construction test
        let bids = vec![];
        let asks = vec![];
        let amm = PoolSnapshot::new(vec![], SqrtPriceX96::from_float_price(0.0)).unwrap();
        OrderBook::new(FixedBytes::<32>::random(), Some(amm), bids, asks, None);
    }
}
