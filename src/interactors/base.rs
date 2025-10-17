pub trait Interactor<Input> {
    type Output;
    type Err;

    async fn execute(&mut self, input: Input) -> Result<Self::Output, Self::Err>;
}
