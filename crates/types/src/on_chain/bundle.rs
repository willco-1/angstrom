use std::{collections::HashMap, hash::Hash};

use alloy_primitives::{Address, U256};
use alloy_rlp::{Decodable, Encodable};
use alloy_rlp_derive::{RlpDecodable, RlpEncodable};
use alloy_sol_types::sol;
use revm::primitives::{TransactTo, TxEnv, U256 as RU256};
use serde::{Deserialize, Serialize};

use crate::{
    contract_bindings::{Angstrom::Order, PoolManager::PoolKey},
    Signature
};

sol! {
    #![sol(all_derives = true)]
    type Currency is address;
    /// @notice Instruction to settle an amount of currency.
    struct CurrencySettlement {
        /// @member The currency to settle.
        Currency currency;
        /// @member The amount to settle, positive indicates we must pay, negative
        ///         indicates we are paid.
        int256 amountNet;
    }

    /// @notice Instruction to donate revenue to a pool.
    #[derive(Debug, PartialEq, Eq)]
    struct PoolFees {
        /// @member The pool to pay fees to.
        PoolKey pool;
        /// @member The amount0 fee.
        uint256 fees0;
        /// @member The amount1 fee.
        uint256 fees1;
    }

    /// @notice Instruction to execute a swap on UniswapV4.
    #[derive(Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
    struct PoolSwap {
        /// @member The pool to perform the swap on.
        PoolKey pool;
        /// @member The input currency.
        Currency currencyIn;
        /// @member The amount of input.
        uint256 amountIn;
    }
    /// @notice Uniswap instructions to execute after lock is taken.
    #[derive(Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
    struct UniswapData {
        /// @member The discrete swaps to perform, there should be at most one entry
        ///         per pool.
        PoolSwap[] swaps;
        /// @member The currency settlements to perform, there should be at most one
        ///         entry per currency.
        CurrencySettlement[] currencies;
        /// @member The fees to pay to each pool, there should be at most one entry
        ///         per pool.
        PoolFees[] pools;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedVanillaBundle {
    pub bundle:     VanillaBundle,
    pub signatures: Vec<Signature>
}

#[derive(Debug, Clone, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct VanillaBundle {
    orders:       Vec<Order>,
    uniswap_data: UniswapData
}

impl VanillaBundle {
    pub fn new(orders: Vec<Order>, uniswap_data: UniswapData) -> anyhow::Result<Self> {
        let mev_bundle = orders
            .iter()
            .find(|order| !order.preHook.is_empty() || !order.postHook.is_empty());

        if mev_bundle.is_some() {
            anyhow::bail!("found a non_villa order: {:?}", mev_bundle);
        }

        Ok(Self { orders, uniswap_data })
    }
}

impl From<VanillaBundle> for TxEnv {
    fn from(value: VanillaBundle) -> Self {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MevBundle {
    pub orders:       Vec<Order>,
    pub uniswap_data: UniswapData
}

impl From<MevBundle> for TxEnv {
    fn from(value: MevBundle) -> Self {
        todo!()
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CallerInfo {
    pub address:   Address,
    pub nonce:     u64,
    pub overrides: HashMap<Address, HashMap<U256, U256>>
}
