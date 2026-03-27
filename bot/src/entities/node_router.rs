#[derive(Debug)]
pub struct NodeStats<'a> {
    pub name: &'a str,
    pub active_downloads: u32,
    pub max_concurrent: u32,
}
