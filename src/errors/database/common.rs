use sea_orm::DbErr;

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

#[derive(Debug, thiserror::Error)]
pub enum ErrorKind<E> {
    #[error(transparent)]
    Expected(E),
    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[macro_export]
macro_rules! impl_from_unexpected_error {
    ($($err_type:ty),*) => {
        $(
            impl<E> From<$err_type> for ErrorKind<E> {
                fn from(err: $err_type) -> Self {
                    ErrorKind::Unexpected(err.into())
                }
            }
        )*
    };
}

impl_from_unexpected_error!(BeginError);
impl_from_unexpected_error!(CommitError);
impl_from_unexpected_error!(RollbackError);
impl_from_unexpected_error!(TransactionNotBegin);
impl_from_unexpected_error!(DbErr);
