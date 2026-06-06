#[derive(Debug)]
pub struct NodeStats {
    pub name: Box<str>,
    pub active_downloads: u32,
    pub max_concurrent: u32,
}
