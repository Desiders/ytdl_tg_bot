const ASPECT_VERTICAL: f32 = 9.0 / 16.0;
const ASPECT_SD: f32 = 4.0 / 3.0;
const ASPECT_HD: f32 = 16.0 / 9.0;

#[derive(Debug, Clone, Copy)]
pub enum AspectKind {
    Vertical,
    Sd,
    Hd,
    Other,
}

impl AspectKind {
    pub const fn new(aspect_ratio: f32) -> Self {
        if aspect_is_equal(aspect_ratio, ASPECT_VERTICAL) {
            return AspectKind::Vertical;
        }
        if aspect_is_equal(aspect_ratio, ASPECT_SD) {
            return AspectKind::Sd;
        }
        if aspect_is_equal(aspect_ratio, ASPECT_HD) {
            return AspectKind::Hd;
        }
        AspectKind::Other
    }
}

const fn aspect_is_equal(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}
