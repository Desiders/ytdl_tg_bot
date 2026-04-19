use telers::event::simple::HandlerResult;

#[allow(clippy::module_name_repetitions)]
pub async fn on_shutdown() -> HandlerResult {
    Ok(())
}
