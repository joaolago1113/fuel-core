//! Types related to executor service.

use crate::{
    blockchain::{
        block::{
            Block,
            PartialFuelBlock,
        },
        primitives::BlockId,
    },
    fuel_tx::{
        TxId,
        UtxoId,
        ValidityError,
    },
    fuel_types::{
        Bytes32,
        ContractId,
        Nonce,
    },
    fuel_vm::{
        checked_transaction::CheckError,
        Backtrace,
        InterpreterError,
        ProgramState,
    },
    services::Uncommitted,
};
use std::error::Error as StdError;

/// The alias for executor result.
pub type Result<T> = core::result::Result<T, Error>;
/// The uncommitted result of the transaction execution.
pub type UncommittedResult<DatabaseTransaction> =
    Uncommitted<ExecutionResult, DatabaseTransaction>;

/// The result of transactions execution.
#[derive(Debug)]
pub struct ExecutionResult {
    /// Created block during the execution of transactions. It contains only valid transactions.
    pub block: Block,
    /// The list of skipped transactions with corresponding errors. Those transactions were
    /// not included in the block and didn't affect the state of the blockchain.
    pub skipped_transactions: Vec<(TxId, Error)>,
    /// The status of the transactions execution included into the block.
    pub tx_status: Vec<TransactionExecutionStatus>,
}

/// The status of a transaction after it is executed.
#[derive(Debug, Clone)]
pub struct TransactionExecutionStatus {
    /// The id of the transaction.
    pub id: Bytes32,
    /// The result of the executed transaction.
    pub result: TransactionExecutionResult,
}

/// The result of transaction execution.
#[derive(Debug, Clone)]
pub enum TransactionExecutionResult {
    /// Transaction was successfully executed.
    Success {
        /// The result of successful transaction execution.
        result: Option<ProgramState>,
    },
    /// The execution of the transaction failed.
    Failed {
        /// The result of failed transaction execution.
        result: Option<ProgramState>,
        /// The reason of execution failure.
        reason: String,
    },
}

/// Execution wrapper where the types
/// depend on the type of execution.
#[derive(Debug, Clone, Copy)]
pub enum ExecutionTypes<P, V> {
    /// DryRun mode where P is being produced.
    DryRun(P),
    /// Production mode where P is being produced.
    Production(P),
    /// Validation mode where V is being checked.
    Validation(V),
}

/// Starting point for executing a block. Production starts with a [`PartialFuelBlock`].
/// Validation starts with a full [`FuelBlock`].
pub type ExecutionBlock = ExecutionTypes<PartialFuelBlock, Block>;

impl<P> ExecutionTypes<P, Block> {
    /// Get the hash of the full [`FuelBlock`] if validating.
    pub fn id(&self) -> Option<BlockId> {
        match self {
            ExecutionTypes::DryRun(_) => None,
            ExecutionTypes::Production(_) => None,
            ExecutionTypes::Validation(v) => Some(v.id()),
        }
    }
}

// TODO: Move `ExecutionType` and `ExecutionKind` into `fuel-core-executor`

/// Execution wrapper with only a single type.
pub type ExecutionType<T> = ExecutionTypes<T, T>;

impl<P, V> ExecutionTypes<P, V> {
    /// Map the production type if producing.
    pub fn map_p<Q, F>(self, f: F) -> ExecutionTypes<Q, V>
    where
        F: FnOnce(P) -> Q,
    {
        match self {
            ExecutionTypes::DryRun(p) => ExecutionTypes::DryRun(f(p)),
            ExecutionTypes::Production(p) => ExecutionTypes::Production(f(p)),
            ExecutionTypes::Validation(v) => ExecutionTypes::Validation(v),
        }
    }

    /// Map the validation type if validating.
    pub fn map_v<W, F>(self, f: F) -> ExecutionTypes<P, W>
    where
        F: FnOnce(V) -> W,
    {
        match self {
            ExecutionTypes::DryRun(p) => ExecutionTypes::DryRun(p),
            ExecutionTypes::Production(p) => ExecutionTypes::Production(p),
            ExecutionTypes::Validation(v) => ExecutionTypes::Validation(f(v)),
        }
    }

    /// Get a reference version of the inner type.
    pub fn as_ref(&self) -> ExecutionTypes<&P, &V> {
        match *self {
            ExecutionTypes::DryRun(ref p) => ExecutionTypes::DryRun(p),
            ExecutionTypes::Production(ref p) => ExecutionTypes::Production(p),
            ExecutionTypes::Validation(ref v) => ExecutionTypes::Validation(v),
        }
    }

    /// Get a mutable reference version of the inner type.
    pub fn as_mut(&mut self) -> ExecutionTypes<&mut P, &mut V> {
        match *self {
            ExecutionTypes::DryRun(ref mut p) => ExecutionTypes::DryRun(p),
            ExecutionTypes::Production(ref mut p) => ExecutionTypes::Production(p),
            ExecutionTypes::Validation(ref mut v) => ExecutionTypes::Validation(v),
        }
    }

    /// Get the kind of execution.
    pub fn to_kind(&self) -> ExecutionKind {
        match self {
            ExecutionTypes::DryRun(_) => ExecutionKind::DryRun,
            ExecutionTypes::Production(_) => ExecutionKind::Production,
            ExecutionTypes::Validation(_) => ExecutionKind::Validation,
        }
    }
}

impl<T> ExecutionType<T> {
    /// Map the wrapped type.
    pub fn map<U, F>(self, f: F) -> ExecutionType<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            ExecutionTypes::DryRun(p) => ExecutionTypes::DryRun(f(p)),
            ExecutionTypes::Production(p) => ExecutionTypes::Production(f(p)),
            ExecutionTypes::Validation(v) => ExecutionTypes::Validation(f(v)),
        }
    }

    /// Filter and map the inner type.
    pub fn filter_map<U, F>(self, f: F) -> Option<ExecutionType<U>>
    where
        F: FnOnce(T) -> Option<U>,
    {
        match self {
            ExecutionTypes::DryRun(p) => f(p).map(ExecutionTypes::DryRun),
            ExecutionTypes::Production(p) => f(p).map(ExecutionTypes::Production),
            ExecutionTypes::Validation(v) => f(v).map(ExecutionTypes::Validation),
        }
    }

    /// Get the inner type.
    pub fn into_inner(self) -> T {
        match self {
            ExecutionTypes::DryRun(t)
            | ExecutionTypes::Production(t)
            | ExecutionTypes::Validation(t) => t,
        }
    }

    /// Split into the execution kind and the inner type.
    pub fn split(self) -> (ExecutionKind, T) {
        let kind = self.to_kind();
        (kind, self.into_inner())
    }
}

impl<T> core::ops::Deref for ExecutionType<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            ExecutionTypes::DryRun(p) => p,
            ExecutionTypes::Production(p) => p,
            ExecutionTypes::Validation(v) => v,
        }
    }
}

impl<T> core::ops::DerefMut for ExecutionType<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            ExecutionTypes::DryRun(p) => p,
            ExecutionTypes::Production(p) => p,
            ExecutionTypes::Validation(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The kind of execution.
pub enum ExecutionKind {
    /// Dry run a block.
    DryRun,
    /// Producing a block.
    Production,
    /// Validating a block.
    Validation,
}

impl ExecutionKind {
    /// Wrap a type in this execution kind.
    pub fn wrap<T>(self, t: T) -> ExecutionType<T> {
        match self {
            ExecutionKind::DryRun => ExecutionTypes::DryRun(t),
            ExecutionKind::Production => ExecutionTypes::Production(t),
            ExecutionKind::Validation => ExecutionTypes::Validation(t),
        }
    }
}

#[allow(missing_docs)]
#[derive(Debug, derive_more::Display, derive_more::From)]
#[non_exhaustive]
pub enum Error {
    #[display(fmt = "Transaction id was already used: {_0:#x}")]
    TransactionIdCollision(Bytes32),
    #[display(fmt = "Too many transactions in the block")]
    TooManyTransactions,
    #[display(fmt = "output already exists")]
    OutputAlreadyExists,
    #[display(fmt = "The computed fee caused an integer overflow")]
    FeeOverflow,
    #[display(fmt = "The block is missing `Mint` transaction.")]
    MintMissing,
    #[display(fmt = "Found the second entry of the `Mint` transaction in the block.")]
    MintFoundSecondEntry,
    #[display(fmt = "The `Mint` transaction has an unexpected index.")]
    MintHasUnexpectedIndex,
    #[display(fmt = "The last transaction in the block is not `Mint`.")]
    MintIsNotLastTransaction,
    #[display(fmt = "The `Mint` transaction mismatches expectations.")]
    MintMismatch,
    #[display(fmt = "Can't increase the balance of the coinbase contract: {_0}.")]
    CoinbaseCannotIncreaseBalance(anyhow::Error),
    #[display(fmt = "Coinbase amount mismatches with expected.")]
    CoinbaseAmountMismatch,
    #[from]
    TransactionValidity(TransactionValidityError),
    // TODO: Replace with `fuel_core_storage::Error` when execution error will live in the
    //  `fuel-core-executor`.
    #[display(fmt = "got error during work with storage {_0}")]
    StorageError(anyhow::Error),
    #[display(fmt = "got error during work with relayer {_0}")]
    RelayerError(Box<dyn StdError + Send + Sync>),
    #[display(fmt = "Transaction({transaction_id:#x}) execution error: {error:?}")]
    VmExecution {
        // TODO: Replace with `fuel_core_storage::Error` when execution error will live in the
        //  `fuel-core-executor`.
        error: InterpreterError<anyhow::Error>,
        transaction_id: Bytes32,
    },
    #[display(fmt = "{_0:?}")]
    InvalidTransaction(CheckError),
    #[display(fmt = "Execution error with backtrace")]
    Backtrace(Box<Backtrace>),
    #[display(fmt = "Transaction doesn't match expected result: {transaction_id:#x}")]
    InvalidTransactionOutcome { transaction_id: Bytes32 },
    #[display(fmt = "The amount of charged fees is invalid")]
    InvalidFeeAmount,
    #[display(fmt = "Block id is invalid")]
    InvalidBlockId,
    #[display(fmt = "No matching utxo for contract id ${_0:#x}")]
    ContractUtxoMissing(ContractId),
    #[display(fmt = "message already spent {_0:#x}")]
    MessageAlreadySpent(Nonce),
    #[display(fmt = "Expected input of type {_0}")]
    InputTypeMismatch(String),
}

impl From<Error> for anyhow::Error {
    fn from(error: Error) -> Self {
        anyhow::Error::msg(error)
    }
}

impl From<Backtrace> for Error {
    fn from(e: Backtrace) -> Self {
        Error::Backtrace(Box::new(e))
    }
}

impl From<CheckError> for Error {
    fn from(e: CheckError) -> Self {
        Self::InvalidTransaction(e)
    }
}

impl From<ValidityError> for Error {
    fn from(e: ValidityError) -> Self {
        Self::InvalidTransaction(CheckError::Validity(e))
    }
}

#[allow(missing_docs)]
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum TransactionValidityError {
    #[error("Coin input was already spent")]
    CoinAlreadySpent(UtxoId),
    #[error("Coin has not yet reached maturity")]
    CoinHasNotMatured(UtxoId),
    #[error("The specified coin doesn't exist")]
    CoinDoesNotExist(UtxoId),
    #[error("The specified message was already spent")]
    MessageAlreadySpent(Nonce),
    #[error(
        "Message is not yet spendable, as it's DA height is newer than this block allows"
    )]
    MessageSpendTooEarly(Nonce),
    #[error("The specified message doesn't exist")]
    MessageDoesNotExist(Nonce),
    #[error("The input message sender doesn't match the relayer message sender")]
    MessageSenderMismatch(Nonce),
    #[error("The input message recipient doesn't match the relayer message recipient")]
    MessageRecipientMismatch(Nonce),
    #[error("The input message amount doesn't match the relayer message amount")]
    MessageAmountMismatch(Nonce),
    #[error("The input message nonce doesn't match the relayer message nonce")]
    MessageNonceMismatch(Nonce),
    #[error("The input message data doesn't match the relayer message data")]
    MessageDataMismatch(Nonce),
    #[error("Contract output index isn't valid: {0:#x}")]
    InvalidContractInputIndex(UtxoId),
    #[error("The transaction contains predicate inputs which aren't enabled: {0:#x}")]
    PredicateExecutionDisabled(TxId),
    #[error(
    "The transaction contains a predicate which failed to validate: TransactionId({0:#x})"
    )]
    InvalidPredicate(TxId),
    #[error("Transaction validity: {0:#?}")]
    Validation(CheckError),
}

impl From<CheckError> for TransactionValidityError {
    fn from(e: CheckError) -> Self {
        Self::Validation(e)
    }
}

impl From<ValidityError> for TransactionValidityError {
    fn from(e: ValidityError) -> Self {
        Self::Validation(CheckError::Validity(e))
    }
}
