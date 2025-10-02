pub trait Interactor {
    type Input;
    type Output;
    type Err;

    async fn execute(&mut self, input: Self::Input) -> Result<Self::Output, Self::Err>;
}
