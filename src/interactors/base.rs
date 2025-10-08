pub trait Interactor {
    type Input<'a>;
    type Output;
    type Err;

    async fn execute(&mut self, input: Self::Input<'_>) -> Result<Self::Output, Self::Err>;
}
