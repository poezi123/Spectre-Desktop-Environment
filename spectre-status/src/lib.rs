pub mod battery;
pub mod brightness;
pub mod sound;

pub use battery::Battery;
pub use brightness::Brightness;
pub use sound::Sound;

use std::process::{Command, Stdio};

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn just_run(program: &str, args: &[&str]) -> bool {
    let started = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match started {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}
