use std::cmp::Ordering;

#[derive(Clone, Copy)]
pub(crate) struct NodeSnapshot<'a> {
    pub(crate) index: usize,
    pub(crate) address: &'a str,
    pub(crate) max_concurrent: u32,
    pub(crate) estimated_active_downloads: u32,
}

pub(crate) fn select_best_index(candidates: Vec<NodeSnapshot<'_>>) -> Option<usize> {
    candidates.into_iter().min_by(compare_nodes).map(|node| node.index)
}

fn compare_nodes(left: &NodeSnapshot<'_>, right: &NodeSnapshot<'_>) -> Ordering {
    let left_active = left.estimated_active_downloads;
    let right_active = right.estimated_active_downloads;

    let left_projected = (left_active + 1) * right.max_concurrent;
    let right_projected = (right_active + 1) * left.max_concurrent;

    left_projected
        .cmp(&right_projected)
        .then_with(|| left_active.cmp(&right_active))
        .then_with(|| right.max_concurrent.cmp(&left.max_concurrent))
        .then_with(|| left.address.cmp(right.address))
}
