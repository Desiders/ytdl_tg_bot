pub trait Interactor {
    type Input<'a>;
    type Output;
    type Err;

    async fn execute<'a>(&mut self, input: Self::Input<'a>) -> Result<Self::Output, Self::Err>;
}
