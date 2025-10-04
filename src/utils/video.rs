const ASPECT_VERTICAL: f64 = 9.0 / 16.0;
const ASPECT_SD: f64 = 4.0 / 3.0;
const ASPECT_HD: f64 = 16.0 / 9.0;

pub enum AspectKind {
    Vertical,
    Sd,
    Hd,
    Other,
}

pub const fn calculate_aspect_ratio(width: Option<i64>, height: Option<i64>) -> f64 {
    match (width, height) {
        (Some(width), Some(height)) if height > 0 => width as f64 / height as f64,
        _ => 0.0,
    }
}

const fn aspect_is_equal(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

pub const fn get_nearest_to_aspect(aspect_ratio: f64) -> AspectKind {
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
