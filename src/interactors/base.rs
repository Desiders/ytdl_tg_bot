pub trait Interactor<Input> {
    type Output;
    type Err;

    async fn execute(self, input: Input) -> Result<Self::Output, Self::Err>;
}
