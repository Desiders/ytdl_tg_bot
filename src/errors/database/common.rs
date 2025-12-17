use sea_orm::DbErr;

use crate::impl_from_unexpected_error;

#[derive(Debug, thiserror::Error)]
#[error("Begin error: {0}")]
pub struct BeginError(#[from] pub DbErr);

#[derive(Debug, thiserror::Error)]
#[error("Commit error: {0}")]
pub struct CommitError(#[from] pub DbErr);

#[derive(Debug, thiserror::Error)]
#[error("Rollback error: {0}")]
pub struct RollbackError(#[from] pub DbErr);

#[derive(Debug, thiserror::Error)]
#[error("Transaction not begin")]
pub struct TransactionNotBegin;

impl_from_unexpected_error!(BeginError);
impl_from_unexpected_error!(CommitError);
impl_from_unexpected_error!(RollbackError);
impl_from_unexpected_error!(TransactionNotBegin);
impl_from_unexpected_error!(DbErr);
