//! Locating programs on `PATH` without invoking a shell.

use std::path::{Path, PathBuf};

/// Finds `program` on `PATH`, or validates it when it is already a path.
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let program = program.trim();
    if program.is_empty() {
        return None;
    }
    if program.contains(std::path::MAIN_SEPARATOR) || program.contains('/') {
        let path = crate::paths::expand_home(Path::new(program));
        return is_executable(&path).then_some(path);
    }
    let search_path = std::env::var_os("PATH")?;
    std::env::split_paths(&search_path)
        .flat_map(|directory| candidates(&directory, program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(windows)]
fn candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
    let mut paths = vec![directory.join(program)];
    paths.extend(
        extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| directory.join(format!("{program}{extension}"))),
    );
    paths
}

#[cfg(not(windows))]
fn candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    vec![directory.join(program)]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const EXECUTE_BITS: u32 = 0o111;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & EXECUTE_BITS != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_common_shell() {
        if cfg!(unix) {
            assert!(find_on_path("sh").is_some());
        }
    }

    #[test]
    fn missing_program_is_none() {
        assert!(find_on_path("os-int-no-such-program-xyz").is_none());
        assert!(find_on_path("").is_none());
    }
}
