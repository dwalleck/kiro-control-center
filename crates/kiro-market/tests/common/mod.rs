use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Return the path to the compiled `kiro-market` binary.
///
/// `CARGO_BIN_EXE_kiro-market` is set by cargo when running integration tests.
pub fn get_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kiro-market"))
}

/// Run `kiro-market` in the given directory with `XDG_DATA_HOME` overridden to
/// isolate the cache from the real user data directory.
pub fn run_in_dir(dir: &Path, args: &[&str]) -> Output {
    Command::new(get_binary())
        .args(args)
        .current_dir(dir)
        .env("XDG_DATA_HOME", dir.join(".data"))
        .output()
        .expect("Failed to execute kiro-market")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

pub mod fixtures;
