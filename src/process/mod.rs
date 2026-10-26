//! Shared subprocess lifecycle and bounded output capture.
pub(crate) mod output;
pub(crate) mod tasks;
use std::process::Child;

pub(crate) fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    // The compiler owns a new process group. Kill its descendants too, so TeX
    // engines spawned by latexmk cannot outlive a cancelled build.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}
