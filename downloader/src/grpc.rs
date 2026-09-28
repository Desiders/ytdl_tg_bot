pub mod auth;
pub mod capabilities;
pub mod cookie_manager;
pub mod downloader;
pub mod music_resolver;
pub mod song_recognizer;

use tokio::sync::Semaphore;

/// Derives the current admitted workload from the node's capacity authority.
pub fn active_downloads(semaphore: &Semaphore, max_concurrent: u32) -> u32 {
    let available = u32::try_from(semaphore.available_permits()).unwrap_or(u32::MAX);
    max_concurrent.saturating_sub(available)
}

#[cfg(test)]
mod tests {
    use super::active_downloads;
    use tokio::sync::Semaphore;

    #[test]
    fn active_downloads_are_derived_from_the_capacity_semaphore() {
        let semaphore = Semaphore::new(3);
        assert_eq!(active_downloads(&semaphore, 3), 0);
        let _first = semaphore.try_acquire().unwrap();
        let _second = semaphore.try_acquire().unwrap();
        assert_eq!(active_downloads(&semaphore, 3), 2);
    }
}
