# Angstrom Payload Struct Specification

**Corresponding Source file:** [`Angstrom.sol`](../../contracts/src/Angstrom.sol)

## Overview

The `Angstrom` contract is the core entry point for nodes submitting bundles of orders. The contract
is in charge of:
- validating orders (signatures, nonces, limits)
- settling orders
- enforcing key protocol invariants
    - uniform clearing (all orders for the same pair get the same price)
    - only active nodes are allowed to submit bundles
    - there may only be 1 bundle / block
- interacting with Uniswap V4
    - swapping against Angstrom pools
    - transferring tokens between Uniswap and itself
    - flash borrowing from Uniswap to ensure orders can be filled

Note that the protocol only enforces *some* protocol invariants and leaves others to the economic
security of the nodes to save on gas e.g. checking that prices are consistent across alternative
pairs.

The main entry point for submitting a bundle to the `Angstrom` contract is the `execute(bytes)`
method which takes the encoded bundle.

## Bundle specification & `execute(bytes)`

**Entry-point Control Flow**

When `execute` is called all it does is authenticate that the caller is a node before calling into Uniswap's
`unlock` function which in turn calls **back into** the `Angstrom` contract passing it the original
payload as `data` (this back and forth is required because it establishes a call context from which
we're allowed to actually interact with Uniswap at all).

### Shared Structs & Types

#### `Signature`


```rust
enum Signature {
    Contract {
        from: address,
        signature: List<bytes1>
    },
    Ecdsa {
        v: uint8,
        r: bytes32,
        s: bytes32
    }
}
```

The `Signature` enum represents either an EOA ECDSA signature or a smart contract signature. Ecdsa
signature do not need the signer address specified as the signer can be recovered as part of the
signature validation process.

### Bundle contents

The bundle is a struct encoded using the [PADE format](./Encoding.md). The string of bytes is the
value that the `execute` method is called with (not that the call itself is still ABI encoded, just
the contents of the single `bytes` parameter itself is PADE).


#### Top-level Payload Contents (the `Bundle` struct)

The `data` payload is the PADE encoding of the following struct:

```rust
struct Bundle {
    assets: List<Asset>,
    pairs: List<Pair>,
    pool_updates: List<PoolUpdate>,
    top_of_block_orders: List<TopOfBlockOrder>,
    user_orders: List<UserOrder>,
}
```

#### `Asset`

Solidity: [decoding implementation](../../contracts/src/types/Asset.sol) | [reference
encoding (`src/reference/Asset.sol`)](../../contracts/src/reference/Asset.sol)

```rust
struct Asset {
    addr: address,
    take: u128,
    save: u128,
    settle: u128
}
```

The array of assets specifies what assets are going to be traded or settled in the submitted bundle.
The elements **must be** sorted in ascending order according the value of `.addr`.

|Field|Description|
|-----|-----------|
|`addr: address`|Contract address of ERC20 token of the asset. |
|`take: uint128`|Amount of the asset to take from Uniswap (`.addr` base unit) |
|`save: uint128`|Amount of the asset to save as the network fee (`.addr` base unit)|
|`settle: uint128`|Final amount to be repayed to Uniswap post-bundle execution. (`.addr` base unit)|

#### `Pair`

Solidity: [decoding implementation](../../contracts/src/types/Pair.sol) | [reference
encoding (`src/reference/Pair.sol`)](../../contracts/src/reference/Pair.sol)

```rust
struct Pair {
    index_a: u16,
    index_b: u16,
    price_AOverB: u256
}
```

This list of pairs defines the unique uniform clearing prices for the different pairs in the bundle.
The elements **must be** sorted in ascending order according to the tuple `(.index_a, .index_b)`.

Note that to ensure pair uniqueness `.index_a` **must** be less than `.index_b`.

|Field|Description|
|-----|-----------|
|`index_a: u16`|Pair's asset A as index into the asset array|
|`index_b: u16`|Pair's asset B as index into the asset array|
|`price_AOverB: u256`|Uniform clearing price of pair in asset A **over** asset B base units in Ray e.g. `13.2e27` represents 13.2 base units of A for every base unit of A.|



#### `PoolUpdate`

TODO: Update solidity link

Solidity: [decoding implementation](../../contracts/src/types/PoolSwap.sol) | [reference
encoding (`src/reference/PoolSwap.sol`)](../../contracts/src/reference/PoolSwap.sol)


```rust
struct PoolUpdate {
    asset_in_index: u16,
    asset_out_index: u16,
    swap_in_quantity: u128,
    rewards_update: RewardsUpdate
}
```

The array of `PoolUpdate`s represents intended changes to the underlying pool. This includes the
netted out swap to execute against the underlying Uniswap pool (where Angstrom is its hook) and
rewards to be saved & accounted for LPs. The contract does not enforce update uniqueness, it is
recommended to net out multiple swaps against the same pool into one to save on gas.

|Field|Description|
|-----|-----------|
|`asset_in_index: u16`|Swap's input asset as index into the assets array|
|`asset_out_index: u16`|Swap's output asset as index into the assets array|
|`swap_in_quantity: u128`|The swap input quantity in the input asset's base units.|
|`rewards_update: RewardsUpdate`| The pool's LP rewards to distribute *after* the pool swap is executed  |

##### Rewards Update

Solidity: [decoding implementation (`_decodeAndReward`)](../../contracts/src/modules/RewardsUpdater.sol) | [reference encoding](../../contracts/src/reference/PoolRewardsUpdate.sol).

```rust
struct RewardsUpdate {
    below: bool,
    start_tick: i24,
    start_liquidity: u128,
    quantities: List<u128>
}
```

The rewards update data informs the Angstrom contract where to begin the reward update loop
(`startTick`) and what quantities to donate. The `start_liquidity` value informs the contract what
the total liquidity is at the start as this cannot be efficiently querried. Note that the value is
eventually checked however and is computed as the loop progresses to ensure consistency of reward
distribution.

To donate only to the current tick one must set a `start_tick` that is itself already out-of-bounds
(for `below = true` this is any tick `> current_tick`, for `below = false` this is nay tick `<=
current_tick`).

|Field|Description |
|-----|-----------|
|`below: bool`| Whether the reward update starts below or above the current tick.|
|`start_tick: i24`| When `below = true` the current tick: the first tick **above** the first tick to donate to. <br> When rewarding above: just the first tick actually being donated to. |
|`start_liquidity: u128`|The current liquidity if `start_tick` were the current tick.|
|`quantities: List<u128>`|The reward for each initialized tick range *including* the current tick in
`asset0` base units.|

**Reward Update Internals**

To gain a better intuition over how parameters need to be set it's good to understand how the reward
update loop operates. The main purpose of the loop is to update the _internal_
`reward_growth_outside` value of the ticks it passes. The "growth outside" values represent
cumulative rewards in such a way where the total rewards accrued by a liquidity range can be
efficiently computed later.

To understand why rewarding above and below the current tick requires different starting values
we need to understand how the meaning of a tick's `reward_growth_outside` value changes
_relative to the current tick_:

![](./assets/fee-growth-outside-meaning.png)

For ticks that are **below or at** the current tick their `reward_growth_outside` value represents the sum
of all accrued rewards in the ticks below **excluding the tick's own rewards**.

Conversely for ticks that are above the current tick their `reward_growth_outside` value represents
the sum of all accrued rewards in the ticks above **including its own**.

Generally the logic for updating pool rewards looks as follows:

```python
def update_rewards(
    pool: Pool,
    start_tick: Tick,
    quantities: list[int],
    liquidity: int,
    below: bool
):
    cumulative_reward_growth: float = 0

    end_tick: Tick = get_current_tick()
    ticks: list[Tick] = initialized_tick_range(start_tick, end_tick, include_end=below)

    for tick, quantity in zip(ticks, quantities):
        cumulative_reward_growth += quantity / liquidity
        tick.reward_growth_outside += cumulative_reward_growth

        if below:
            liquidity += tick.net_liquidity
        else:
            liquidity -= tick.net_liquidity

    assert len(quantities) == len(ticks) + 1, 'Unused quantities'

    current_tick_reward: int = quantities[len(ticks)]
    cumulative_reward_growth += current_tick_reward / liquidity
    pool.global_reward_growth += sum(quantities)

    assert liquidity == get_current_liquidity(), 'Invalid set start liquidity'
```


#### `TopOfBlockOrder`

Solidity: [decoding implementation (`_validateAndExecuteToB`)](../../contracts/src/Angstrom.sol) | [reference encoding](../../contracts/src/reference/OrderTypes.sol).

```rust
struct TopOfBlockOrder {
    use_internal: bool,
    quantity_in: u128,
    quantity_out: u128,
    asset_in_index: u16,
    asset_out_index: u16,
    recipient: Option<address>,
    hook_data: Option<List<bytes1>>,
    signature: Signature
}
```

|Field|Description|
|-----|-----------|
|`use_internal: bool`|Whether to use angstrom internal balance (`true`) or actual ERC20 balance (`false`) to settle|
|`quantity_in: u128`|The order offered input quanity in the input asset's base units.|
|`quantity_out: u128`|The order expected output quantity in the output asset's base units.|
|`asset_in_index: u16`|Order's input asset as index into the assets array|
|`asset_out_index: u16`|Order's output asset as index into the assets array|
|`recipient: Option<address>`|Recipient for order output, `None` implies signer.|
|`hook_data: Option<List<bytes1>>`|Optional hook for composable orders, consisting of the hook address concatenated to the hook extra data.|
|`signature: Signature`|The signature validating the order.|

#### `UserOrder`

```rust
struct UserOrder {
    use_interal: bool,
    pair_index: u16,
    min_price: u256,
    recipient: Option<address>,
    hook_data: Option<List<bytes1>>,
    a_to_b: bool,
    standing_validation: Option<StandingValidation>,
    order_quantities: OrderQuantities,
    exact_in: bool,
    signature: Signature
}

struct StandingValidation {
    nonce: u64,
    deadline: u40
}

enum OrderQuantities {
    Exact {
        quantity: u128
    },
    Partial {
        min_quantity_in: u128,
        max_quantity_in: u128,
        filled_quantity: u128
    }
}
```

**`UserOrder`**

|Field|Description|
|-----|-----------|
|`use_internal: bool`|Whether to use angstrom internal balance (`true`) or actual ERC20 balance (`false`) to settle|
|`pair_index: u16`|The index into the `List<Pair>` array that the order is trading in.|
|`min_price: u256`|The minimum price in asset out over asset in base units in RAY|
|`recipient: Option<address>`|Recipient for order output, `None` implies signer.|
|`hook_data: Option<List<bytes1>>`|Optional hook for composable orders, consisting of the hook address concatenated to the hook extra data.|
|`a_to_b: bool`|Whether the order is swapping in the pair's asset A and getting out B (`true`) or the other way around (`false`)|
|`standing_validation: Option<StandingValidation>`|The one-time order validation data. (`None` implies a flash order which is validated via the block number)|
|`order_quantities: OrderQuantities`|Description of the quantities the order trades.|
|`exact_in: bool`|For exact orders: whether the specified quantity is the input or output (disregarded for partial orders).|
|`signature: Signature`|The signature validating the order.|

**`StandingValidation`**
|Field|Description|
|-----|-----------|
|`nonce: u64`|The order's nonce (can only be used once but do not have to be used in order).|
|`deadline: u40`|The unix timestamp in seconds (inclusive) after which the order is considered invalid by the contract. |