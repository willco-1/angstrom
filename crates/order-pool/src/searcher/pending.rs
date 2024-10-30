use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap}
};

use alloy::primitives::FixedBytes;
use angstrom_types::{
    orders::OrderPriorityData,
    sol_bindings::{grouped_orders::OrderWithStorageData, rpc_orders::TopOfBlockOrder}
};

pub struct PendingPool {
    /// all order hashes
    orders: HashMap<FixedBytes<32>, OrderWithStorageData<TopOfBlockOrder>>,
    /// bids are sorted descending by price, TODO: This should be binned into
    /// ticks based off of the underlying pools params
    bids:   BTreeMap<Reverse<OrderPriorityData>, FixedBytes<32>>,
    /// asks are sorted ascending by price,  TODO: This should be binned into
    /// ticks based off of the underlying pools params
    asks:   BTreeMap<OrderPriorityData, FixedBytes<32>>
}

impl PendingPool {
    #[allow(unused)]
    pub fn new() -> Self {
        Self { orders: HashMap::new(), bids: BTreeMap::new(), asks: BTreeMap::new() }
    }

    pub fn get_order(&self, id: FixedBytes<32>) -> Option<OrderWithStorageData<TopOfBlockOrder>> {
        self.orders.get(&id).cloned()
    }

    pub fn add_order(&mut self, order: OrderWithStorageData<TopOfBlockOrder>) {
        if order.is_bid {
            self.bids
                .insert(Reverse(order.priority_data), order.order_id.hash);
        } else {
            self.asks.insert(order.priority_data, order.order_id.hash);
        }
        self.orders.insert(order.order_id.hash, order);
    }

    pub fn remove_order(
        &mut self,
        id: FixedBytes<32>
    ) -> Option<OrderWithStorageData<TopOfBlockOrder>> {
        let order = self.orders.remove(&id)?;

        if order.is_bid {
            self.bids.remove(&Reverse(order.priority_data))?;
        } else {
            self.asks.remove(&order.priority_data)?;
        }

        // probably fine to strip extra data here
        Some(order)
    }

    pub fn get_all_orders(&self) -> Vec<OrderWithStorageData<TopOfBlockOrder>> {
        // TODO:  This should maybe only return the one best Searcher order we've seen?
        self.orders.values().cloned().collect()
    }

    pub fn get_best_order(&self) -> Option<OrderWithStorageData<TopOfBlockOrder>> {
        self.bids
            .keys()
            .next()
            .and_then(|key| self.orders.get(&self.bids[key]))
            .cloned()
    }

    pub fn bids_and_asks_to_ticks(
        &self,
        tick_size: alloy::primitives::U256,
    ) -> (BTreeMap<alloy::primitives::U256, Vec<FixedBytes<32>>>, BTreeMap<alloy::primitives::U256, Vec<FixedBytes<32>>>) {
        let mut bid_ticks: BTreeMap<alloy::primitives::U256, Vec<FixedBytes<32>>> = BTreeMap::new();
        let mut ask_ticks: BTreeMap<alloy::primitives::U256, Vec<FixedBytes<32>>> = BTreeMap::new();

        let price_to_tick = |price: alloy::primitives::U256| -> alloy::primitives::U256 { price / tick_size };

        for (Reverse(priority_data), order_hash) in &self.bids {
            let tick_index = price_to_tick(priority_data.price);
            bid_ticks
                .entry(tick_index)
                .or_insert_with(Vec::new)
                .push(*order_hash);
        }

        for (priority_data, order_hash) in &self.asks {
            let tick_index = price_to_tick(priority_data.price);
            ask_ticks
                .entry(tick_index)
                .or_insert_with(Vec::new)
                .push(*order_hash);
        }

        (bid_ticks, ask_ticks)
    }
}
