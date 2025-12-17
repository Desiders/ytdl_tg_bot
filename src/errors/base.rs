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
            impl<E> From<$err_type> for $crate::errors::ErrorKind<E> {
                fn from(err: $err_type) -> Self {
                    Self::Unexpected(err.into())
                }
            }
        )*
    };
}
