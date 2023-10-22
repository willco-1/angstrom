//! Ethereum transaction validator.

use std::{
    marker::PhantomData,
    sync::{atomic::AtomicBool, Arc}
};

use reth_primitives::{
    constants::{
        eip4844::{MAINNET_KZG_TRUSTED_SETUP, MAX_BLOBS_PER_BLOCK},
        ETHEREUM_BLOCK_GAS_LIMIT
    },
    kzg::KzgSettings,
    revm::compat::calculate_intrinsic_gas_after_merge,
    ChainSpec, InvalidTransactionError, SealedBlock, EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID,
    EIP4844_TX_TYPE_ID, LEGACY_TX_TYPE_ID
};
use reth_provider::{AccountReader, StateProviderFactory};
use reth_tasks::TaskSpawner;
use tokio::sync::Mutex;

use crate::{
    blobstore::BlobStore,
    error::{Eip4844PoolTransactionError, InvalidPoolTransactionError},
    traits::TransactionOrigin,
    validate::{ValidTransaction, ValidationTask, MAX_INIT_CODE_SIZE, TX_MAX_SIZE},
    EthBlobTransactionSidecar, EthPoolTransaction, OrderValidator, PoolTransaction,
    TransactionValidationOutcome, TransactionValidationTaskExecutor
};

/// Validator for Ethereum transactions.
#[derive(Debug, Clone)]
pub struct EthOrderValidator<Client, T> {
    /// The type that performs the actual validation.
    inner: Arc<OrderValidatorInner<Client, T>>
}

impl<Client, Tx> EthOrderValidator<Client, Tx>
where
    Client: StateProviderFactory,
    Tx: EthPoolTransaction
{
    /// Validates a single transaction.
    ///
    /// See also [OrderValidator::validate_transaction]
    pub fn validate_one(
        &self,
        origin: TransactionOrigin,
        transaction: Tx
    ) -> TransactionValidationOutcome<Tx> {
        self.inner.validate_one(origin, transaction)
    }

    /// Validates all given transactions.
    ///
    /// Returns all outcomes for the given transactions in the same order.
    ///
    /// See also [Self::validate_one]
    pub fn validate_all(
        &self,
        transactions: Vec<(TransactionOrigin, Tx)>
    ) -> Vec<TransactionValidationOutcome<Tx>> {
        transactions
            .into_iter()
            .map(|(origin, tx)| self.validate_one(origin, tx))
            .collect()
    }
}

#[async_trait::async_trait]
impl<Client, Tx> OrderValidator for EthOrderValidator<Client, Tx>
where
    Client: StateProviderFactory,
    Tx: EthPoolTransaction
{
    type Transaction = Tx;

    async fn validate_transaction(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction
    ) -> TransactionValidationOutcome<Self::Transaction> {
        self.validate_one(origin, transaction)
    }

    async fn validate_transactions(
        &self,
        transactions: Vec<(TransactionOrigin, Self::Transaction)>
    ) -> Vec<TransactionValidationOutcome<Self::Transaction>> {
        self.validate_all(transactions)
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock) {
        self.inner.on_new_head_block(new_tip_block)
    }
}

/// A [OrderValidator] implementation that validates ethereum transaction.
#[derive(Debug)]
pub(crate) struct OrderValidatorInner<Client, T> {
    /// Spec of the chain
    chain_spec: Arc<ChainSpec>,
    /// This type fetches account info from the db
    client: Client,
    /// tracks activated forks relevant for transaction validation
    fork_tracker: ForkTracker,
    /// Fork indicator whether we are using EIP-2718 type transactions.
    eip2718: bool,
    /// Fork indicator whether we are using EIP-1559 type transactions.
    eip1559: bool,
    /// Fork indicator whether we are using EIP-4844 blob transactions.
    eip4844: bool,
    /// The current max gas limit
    block_gas_limit: u64,
    /// Minimum priority fee to enforce for acceptance into the pool.
    minimum_priority_fee: Option<u128>,
    /// Toggle to determine if a local transaction should be propagated
    propagate_local_transactions: bool,
    /// Stores the setup and parameters needed for validating KZG proofs.
    kzg_settings: Arc<KzgSettings>,
    /// Marker for the transaction type
    _marker: PhantomData<T>
}

// === impl OrderValidatorInner ===

impl<Client, Tx> OrderValidatorInner<Client, Tx> {
    /// Returns the configured chain id
    pub(crate) fn chain_id(&self) -> u64 {
        self.chain_spec.chain().id()
    }
}

impl<Client, Tx> OrderValidatorInner<Client, Tx>
where
    Client: StateProviderFactory,
    Tx: EthPoolTransaction
{
    /// Validates a single transaction.
    fn validate_one(
        &self,
        origin: TransactionOrigin,
        mut transaction: Tx
    ) -> TransactionValidationOutcome<Tx> {
        // Checks for tx_type
        match transaction.tx_type() {
            LEGACY_TX_TYPE_ID => {
                // Accept legacy transactions
            }
            EIP2930_TX_TYPE_ID => {
                // Accept only legacy transactions until EIP-2718/2930 activates
                if !self.eip2718 {
                    return TransactionValidationOutcome::Invalid(
                        transaction,
                        InvalidTransactionError::Eip1559Disabled.into()
                    )
                }
            }
            EIP1559_TX_TYPE_ID => {
                // Reject dynamic fee transactions until EIP-1559 activates.
                if !self.eip1559 {
                    return TransactionValidationOutcome::Invalid(
                        transaction,
                        InvalidTransactionError::Eip1559Disabled.into()
                    )
                }
            }
            EIP4844_TX_TYPE_ID => {
                // Reject blob transactions.
                if !self.eip4844 {
                    return TransactionValidationOutcome::Invalid(
                        transaction,
                        InvalidTransactionError::Eip4844Disabled.into()
                    )
                }
            }

            _ => {
                return TransactionValidationOutcome::Invalid(
                    transaction,
                    InvalidTransactionError::TxTypeNotSupported.into()
                )
            }
        };

        // Reject transactions over defined size to prevent DOS attacks
        if transaction.size() > TX_MAX_SIZE {
            let size = transaction.size();
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidPoolTransactionError::OversizedData(size, TX_MAX_SIZE)
            )
        }

        // Check whether the init code size has been exceeded.
        if self.fork_tracker.is_shanghai_activated() {
            if let Err(err) = ensure_max_init_code_size(&transaction, MAX_INIT_CODE_SIZE) {
                return TransactionValidationOutcome::Invalid(transaction, err)
            }
        }

        // Checks for gas limit
        if transaction.gas_limit() > self.block_gas_limit {
            let gas_limit = transaction.gas_limit();
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidPoolTransactionError::ExceedsGasLimit(gas_limit, self.block_gas_limit)
            )
        }

        // Ensure max_priority_fee_per_gas (if EIP1559) is less than max_fee_per_gas if
        // any.
        if transaction.max_priority_fee_per_gas() > Some(transaction.max_fee_per_gas()) {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidTransactionError::TipAboveFeeCap.into()
            )
        }

        // Drop non-local transactions with a fee lower than the configured fee for
        // acceptance into the pool.
        if !origin.is_local()
            && transaction.is_eip1559()
            && transaction.max_priority_fee_per_gas() < self.minimum_priority_fee
        {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidPoolTransactionError::Underpriced
            )
        }

        // Checks for chainid
        if let Some(chain_id) = transaction.chain_id() {
            if chain_id != self.chain_id() {
                return TransactionValidationOutcome::Invalid(
                    transaction,
                    InvalidTransactionError::ChainIdMismatch.into()
                )
            }
        }

        // intrinsic gas checks
        let access_list = transaction
            .access_list()
            .map(|list| list.flattened())
            .unwrap_or_default();
        let is_shanghai = self.fork_tracker.is_shanghai_activated();

        if transaction.gas_limit()
            < calculate_intrinsic_gas_after_merge(
                transaction.input(),
                transaction.kind(),
                &access_list,
                is_shanghai
            )
        {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidPoolTransactionError::IntrinsicGasTooLow
            )
        }

        let mut maybe_blob_sidecar = None;

        // blob tx checks
        if transaction.is_eip4844() {
            // Cancun fork is required for blob txs
            if !self.fork_tracker.is_cancun_activated() {
                return TransactionValidationOutcome::Invalid(
                    transaction,
                    InvalidTransactionError::TxTypeNotSupported.into()
                )
            }

            let blob_count = transaction.blob_count();
            if blob_count == 0 {
                // no blobs
                return TransactionValidationOutcome::Invalid(
                    transaction,
                    InvalidPoolTransactionError::Eip4844(
                        Eip4844PoolTransactionError::NoEip4844Blobs
                    )
                )
            }

            if blob_count > MAX_BLOBS_PER_BLOCK {
                // too many blobs
                return TransactionValidationOutcome::Invalid(
                    transaction,
                    InvalidPoolTransactionError::Eip4844(
                        Eip4844PoolTransactionError::TooManyEip4844Blobs {
                            have:      blob_count,
                            permitted: MAX_BLOBS_PER_BLOCK
                        }
                    )
                )
            }

            // extract the blob from the transaction
            match transaction.take_blob() {
                EthBlobTransactionSidecar::None => {
                    // this should not happen
                    return TransactionValidationOutcome::Invalid(
                        transaction,
                        InvalidTransactionError::TxTypeNotSupported.into()
                    )
                }
                EthBlobTransactionSidecar::Missing => {
                    if let Ok(Some(_)) = self.blob_store.get(*transaction.hash()) {
                        // validated transaction is already in the store
                    } else {
                        return TransactionValidationOutcome::Invalid(
                            transaction,
                            InvalidPoolTransactionError::Eip4844(
                                Eip4844PoolTransactionError::MissingEip4844BlobSidecar
                            )
                        )
                    }
                }
                EthBlobTransactionSidecar::Present(blob) => {
                    if let Some(eip4844) = transaction.as_eip4844() {
                        // validate the blob
                        if let Err(err) = eip4844.validate_blob(&blob, &self.kzg_settings) {
                            return TransactionValidationOutcome::Invalid(
                                transaction,
                                InvalidPoolTransactionError::Eip4844(
                                    Eip4844PoolTransactionError::InvalidEip4844Blob(err)
                                )
                            )
                        }
                        // store the extracted blob
                        maybe_blob_sidecar = Some(blob);
                    } else {
                        // this should not happen
                        return TransactionValidationOutcome::Invalid(
                            transaction,
                            InvalidTransactionError::TxTypeNotSupported.into()
                        )
                    }
                }
            }
        }

        let account = match self
            .client
            .latest()
            .and_then(|state| state.basic_account(transaction.sender()))
        {
            Ok(account) => account.unwrap_or_default(),
            Err(err) => {
                return TransactionValidationOutcome::Error(*transaction.hash(), Box::new(err))
            }
        };

        // Signer account shouldn't have bytecode. Presence of bytecode means this is a
        // smartcontract.
        if account.has_bytecode() {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidTransactionError::SignerAccountHasBytecode.into()
            )
        }

        // Checks for nonce
        if transaction.nonce() < account.nonce {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidTransactionError::NonceNotConsistent.into()
            )
        }

        // Checks for max cost
        let cost = transaction.cost();
        if cost > account.balance {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidTransactionError::InsufficientFunds {
                    cost,
                    available_funds: account.balance
                }
                .into()
            )
        }

        // Return the valid transaction
        TransactionValidationOutcome::Valid {
            balance:     account.balance,
            state_nonce: account.nonce,
            transaction: ValidTransaction::new(transaction, maybe_blob_sidecar),
            // by this point assume all external transactions should be propagated
            propagate:   match origin {
                TransactionOrigin::External => true,
                TransactionOrigin::Local => self.propagate_local_transactions,
                TransactionOrigin::Private => false
            }
        }
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock) {
        // update all forks
        if self
            .chain_spec
            .is_cancun_active_at_timestamp(new_tip_block.timestamp)
        {
            self.fork_tracker
                .cancun
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if self
            .chain_spec
            .is_shanghai_active_at_timestamp(new_tip_block.timestamp)
        {
            self.fork_tracker
                .shanghai
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// A builder for [TransactionValidationTaskExecutor]
#[derive(Debug, Clone)]
pub struct EthOrderValidatorBuilder {
    chain_spec: Arc<ChainSpec>,
    /// Fork indicator whether we are in the Shanghai stage.
    shanghai: bool,
    /// Fork indicator whether we are in the Cancun hardfork.
    cancun: bool,
    /// Whether using EIP-2718 type transactions is allowed
    eip2718: bool,
    /// Whether using EIP-1559 type transactions is allowed
    eip1559: bool,
    /// Whether using EIP-4844 type transactions is allowed
    eip4844: bool,
    /// The current max gas limit
    block_gas_limit: u64,
    /// Minimum priority fee to enforce for acceptance into the pool.
    minimum_priority_fee: Option<u128>,
    /// Determines how many additional tasks to spawn
    ///
    /// Default is 1
    additional_tasks: usize,
    /// Toggle to determine if a local transaction should be propagated
    propagate_local_transactions: bool,

    /// Stores the setup and parameters needed for validating KZG proofs.
    kzg_settings: Arc<KzgSettings>
}

impl EthOrderValidatorBuilder {
    /// Creates a new builder for the given [ChainSpec]
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        // If cancun is enabled at genesis, enable it
        let cancun = chain_spec.is_cancun_active_at_timestamp(chain_spec.genesis_timestamp());

        Self {
            chain_spec,
            block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT,
            minimum_priority_fee: None,
            additional_tasks: 1,
            // default to true, can potentially take this as a param in the future
            propagate_local_transactions: true,
            kzg_settings: Arc::clone(&MAINNET_KZG_TRUSTED_SETUP),

            // by default all transaction types are allowed
            eip2718: true,
            eip1559: true,
            eip4844: true,

            // shanghai is activated by default
            shanghai: true,

            // TODO: can hard enable by default once mainnet transitioned
            cancun
        }
    }

    /// Disables the Cancun fork.
    pub fn no_cancun(self) -> Self {
        self.set_cancun(false)
    }

    /// Set the Cancun fork.
    pub fn set_cancun(mut self, cancun: bool) -> Self {
        self.cancun = cancun;
        self
    }

    /// Disables the Shanghai fork.
    pub fn no_shanghai(self) -> Self {
        self.set_shanghai(false)
    }

    /// Set the Shanghai fork.
    pub fn set_shanghai(mut self, shanghai: bool) -> Self {
        self.shanghai = shanghai;
        self
    }

    /// Disables the eip2718 support.
    pub fn no_eip2718(self) -> Self {
        self.set_eip2718(false)
    }

    /// Set eip2718 support.
    pub fn set_eip2718(mut self, eip2718: bool) -> Self {
        self.eip2718 = eip2718;
        self
    }

    /// Disables the eip1559 support.
    pub fn no_eip1559(self) -> Self {
        self.set_eip1559(false)
    }

    /// Set the eip1559 support.
    pub fn set_eip1559(mut self, eip1559: bool) -> Self {
        self.eip1559 = eip1559;
        self
    }

    /// Sets the [KzgSettings] to use for validating KZG proofs.
    pub fn kzg_settings(mut self, kzg_settings: Arc<KzgSettings>) -> Self {
        self.kzg_settings = kzg_settings;
        self
    }

    /// Sets toggle to propagate transactions received locally by this client
    /// (e.g transactions from eth_sendTransaction to this nodes' RPC
    /// server)
    ///
    ///  If set to false, only transactions received by network peers (via
    /// p2p) will be marked as propagated in the local transaction pool and
    /// returned on a GetPooledTransactions p2p request
    pub fn set_propagate_local_transactions(mut self, propagate_local_txs: bool) -> Self {
        self.propagate_local_transactions = propagate_local_txs;
        self
    }

    /// Disables propagating transactions received locally by this client
    ///
    /// For more information, check docs for set_propagate_local_transactions
    pub fn no_local_transaction_propagation(mut self) -> Self {
        self.propagate_local_transactions = false;
        self
    }

    /// Sets a minimum priority fee that's enforced for acceptance into the
    /// pool.
    pub fn with_minimum_priority_fee(mut self, minimum_priority_fee: u128) -> Self {
        self.minimum_priority_fee = Some(minimum_priority_fee);
        self
    }

    /// Sets the number of additional tasks to spawn.
    pub fn with_additional_tasks(mut self, additional_tasks: usize) -> Self {
        self.additional_tasks = additional_tasks;
        self
    }

    /// Configures validation rules based on the head block's timestamp.
    ///
    /// For example, whether the Shanghai and Cancun hardfork is activated at
    /// launch.
    pub fn with_head_timestamp(mut self, timestamp: u64) -> Self {
        self.cancun = self.chain_spec.is_cancun_active_at_timestamp(timestamp);
        self.shanghai = self.chain_spec.is_shanghai_active_at_timestamp(timestamp);
        self
    }

    /// Builds a the [EthOrderValidator] and spawns validation tasks via the
    /// [TransactionValidationTaskExecutor]
    ///
    /// The validator will spawn `additional_tasks` additional tasks for
    /// validation.
    ///
    /// By default this will spawn 1 additional task.
    pub fn build_with_tasks<Client, Tx, T, S>(
        self,
        client: Client,
        tasks: T,
        blob_store: S
    ) -> TransactionValidationTaskExecutor<EthOrderValidator<Client, Tx>>
    where
        T: TaskSpawner,
        S: BlobStore
    {
        let Self {
            chain_spec,
            shanghai,
            cancun,
            eip2718,
            eip1559,
            eip4844,
            block_gas_limit,
            minimum_priority_fee,
            additional_tasks,
            propagate_local_transactions,
            kzg_settings
        } = self;

        let fork_tracker =
            ForkTracker { shanghai: AtomicBool::new(shanghai), cancun: AtomicBool::new(cancun) };

        let inner = OrderValidatorInner {
            chain_spec,
            client,
            eip2718,
            eip1559,
            fork_tracker,
            eip4844,
            block_gas_limit,
            minimum_priority_fee,
            propagate_local_transactions,
            blob_store: Box::new(blob_store),
            kzg_settings,
            _marker: Default::default()
        };

        let (tx, task) = ValidationTask::new();

        // Spawn validation tasks, they are blocking because they perform db lookups
        for _ in 0..additional_tasks {
            let task = task.clone();
            tasks.spawn_blocking(Box::pin(async move {
                task.run().await;
            }));
        }

        tasks.spawn_critical_blocking(
            "transaction-validation-service",
            Box::pin(async move {
                task.run().await;
            })
        );

        let to_validation_task = Arc::new(Mutex::new(tx));

        TransactionValidationTaskExecutor {
            validator: EthOrderValidator { inner: Arc::new(inner) },
            to_validation_task
        }
    }
}

/// Keeps track of whether certain forks are activated
#[derive(Debug)]
pub(crate) struct ForkTracker {
    /// Tracks if shanghai is activated at the block's timestamp.
    pub(crate) shanghai: AtomicBool,
    /// Tracks if cancun is activated at the block's timestamp.
    pub(crate) cancun:   AtomicBool
}

impl ForkTracker {
    /// Returns true if the Shanghai fork is activated.
    pub(crate) fn is_shanghai_activated(&self) -> bool {
        self.shanghai.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns true if the Shanghai fork is activated.
    pub(crate) fn is_cancun_activated(&self) -> bool {
        self.cancun.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Ensure that the code size is not greater than `max_init_code_size`.
/// `max_init_code_size` should be configurable so this will take it as an
/// argument.
pub fn ensure_max_init_code_size<T: PoolTransaction>(
    transaction: &T,
    max_init_code_size: usize
) -> Result<(), InvalidPoolTransactionError> {
    if transaction.kind().is_create() && transaction.input().len() > max_init_code_size {
        Err(InvalidPoolTransactionError::ExceedsMaxInitCodeSize(
            transaction.size(),
            max_init_code_size
        ))
    } else {
        Ok(())
    }
}