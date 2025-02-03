use std::fmt::Write;
use std::iter;

pub fn format_error_report(err: &impl std::error::Error) -> String {
    let mut output = String::new();
    write!(&mut output, "{}", err).unwrap();

    if let Some(cause) = err.source() {
        write!(&mut output, ". Caused by:").unwrap();
        for (i, err) in iter::successors(Some(cause), |err| err.source()).enumerate() {
            write!(&mut output, " {}: {}", i, err).unwrap();
        }
    }

    output
}
