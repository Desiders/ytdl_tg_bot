use std::{io, process::ExitStatus};

use nix::{
    sys::signal::{killpg, Signal},
    unistd::Pid,
};

/// Owns the process group of an external tool and kills all descendants on drop.
pub struct ProcessGroup(Pid);

impl ProcessGroup {
    #[must_use]
    pub fn new(child: &tokio::process::Child) -> Self {
        Self(Pid::from_raw(
            i32::try_from(child.id().expect("New child has a pid")).expect("Pid fits i32"),
        ))
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = killpg(self.0, Signal::SIGKILL);
    }
}

/// Build an `io::Error` describing a subprocess that exited unsuccessfully.
/// Shared by the yt-dlp and gallery-dl wrappers to keep the messages consistent.
pub fn process_exit_error(name: &str, status: ExitStatus, stderr: &str) -> io::Error {
    match status.code() {
        Some(code) => io::Error::other(format!("{name} exited with code {code} and message: {stderr}")),
        None => io::Error::other(format!("{name} exited with and message: {stderr}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt as _, BufReader};

    #[tokio::test]
    async fn cancellation_kills_descendants_that_inherit_output_pipes() {
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "sleep 60 & echo $!; wait"])
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let group = ProcessGroup::new(&child);
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let descendant: u32 = lines.next_line().await.unwrap().unwrap().parse().unwrap();
        drop(group);
        tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let stat = std::fs::read_to_string(format!("/proc/{descendant}/stat"));
                if stat.as_ref().map_or(true, |stat| stat.split_whitespace().nth(2) == Some("Z")) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(lines.next_line().await.unwrap().is_none());
    }
}
