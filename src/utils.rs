mod error;
mod shutdown;
mod startup;
mod thumbnail;
mod url;
mod video;
mod fs;

pub use error::{format_error_report, FormatErrorToMessage};
pub use shutdown::on_shutdown;
pub use startup::on_startup;
pub use thumbnail::get_urls_by_aspect;
pub use url::{get_video_id, ErrorKind as GetVideoIdErrorKind};
pub use video::{calculate_aspect_ratio, get_nearest_to_aspect, AspectKind};
pub use fs::sanitize_send_filename;