use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let mut path = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("tgrep-cli-test-{}", unique));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn tgrep() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tgrep"))
}

#[test]
fn incompatible_files_without_match_and_invert_match_flags_fail() {
    let temp = TempDir::new();

    let output = tgrep()
        .current_dir(temp.path())
        .args(["-L", "-v", "--no-color", "needle", "."])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("incompatible flags: -L and -v"));
}

#[test]
fn incompatible_count_and_invert_match_flags_fail() {
    let temp = TempDir::new();

    let output = tgrep()
        .current_dir(temp.path())
        .args(["-c", "-v", "--no-color", "needle", "."])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("incompatible flags: -c and -v"));
}

#[test]
fn file_type_filter_limits_results() {
    let temp = TempDir::new();
    temp.write("match.rs", "needle\n");
    temp.write("match.txt", "needle\n");

    let output = tgrep()
        .current_dir(temp.path())
        .args(["-t", "rs", "--no-color", "needle", "."])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("./match.rs:1: needle"));
    assert!(!stdout.contains("match.txt"));
}

#[test]
fn files_without_match_lists_only_non_matching_files() {
    let temp = TempDir::new();
    temp.write("match.txt", "needle\n");
    temp.write("miss.txt", "plain text\n");

    let output = tgrep()
        .current_dir(temp.path())
        .args(["-L", "--no-color", "needle", "."])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("./miss.txt"));
    assert!(!stdout.contains("match.txt"));
}
