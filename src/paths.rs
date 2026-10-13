//! Platform-appropriate locations for configuration, data, logs, and investigation files.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, eyre};
use directories::ProjectDirs;
use uuid::Uuid;

pub const CONFIG_ENV: &str = "OS_INT_CONFIG";
pub const DATA_DIR_ENV: &str = "OS_INT_DATA_DIR";
const APPLICATION_NAME: &str = "os-int";
const CONFIG_FILE_NAME: &str = "config.toml";
const DATABASE_FILE_NAME: &str = "os-int.sqlite3";

/// Resolved filesystem locations used by the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
}

impl AppPaths {
    /// The config file path: CLI flag, then `OS_INT_CONFIG`, then the platform config dir.
    pub fn resolve_config_file(cli_override: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = cli_override {
            return Ok(path.to_path_buf());
        }
        if let Some(path) = std::env::var_os(CONFIG_ENV).filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(path));
        }
        Ok(project_dirs()?.config_dir().join(CONFIG_FILE_NAME))
    }

    /// The data directory: CLI flag, then `OS_INT_DATA_DIR`, then config, then platform default.
    pub fn resolve_data_dir(
        cli_override: Option<&Path>,
        configured: Option<&Path>,
    ) -> Result<PathBuf> {
        if let Some(path) = cli_override {
            return Ok(path.to_path_buf());
        }
        if let Some(path) = std::env::var_os(DATA_DIR_ENV).filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(path));
        }
        if let Some(path) = configured.filter(|path| !path.as_os_str().is_empty()) {
            return Ok(expand_home(path));
        }
        Ok(project_dirs()?.data_dir().to_path_buf())
    }

    pub fn new(config_file: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            config_file,
            data_dir,
        }
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join(DATABASE_FILE_NAME)
    }

    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    pub fn investigations_dir(&self) -> PathBuf {
        self.data_dir.join("investigations")
    }

    pub fn investigation_dir(&self, investigation_id: Uuid) -> PathBuf {
        self.investigations_dir().join(investigation_id.to_string())
    }

    /// Creates the data, log, and cache directories.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            self.data_dir.clone(),
            self.log_dir(),
            self.cache_dir(),
            self.investigations_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .wrap_err_with(|| format!("could not create directory {}", dir.display()))?;
        }
        Ok(())
    }
}

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", APPLICATION_NAME)
        .ok_or_else(|| eyre!("could not determine a home directory for configuration and data"))
}

/// Expands a leading `~/` to the user's home directory.
pub fn expand_home(path: &Path) -> PathBuf {
    let Ok(stripped) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(stripped))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Turns arbitrary text into a safe single path component (`Acme Corp/2024` -> `acme-corp-2024`).
pub fn sanitize_filename(raw: &str) -> String {
    const MAX_FILENAME_LENGTH: usize = 80;
    let mut output = String::with_capacity(raw.len());
    for c in raw.trim().chars() {
        if c.is_alphanumeric() || c == '.' || c == '_' {
            output.extend(c.to_lowercase());
        } else if !output.ends_with('-') {
            output.push('-');
        }
    }
    let trimmed = output.trim_matches(['-', '.']).to_string();
    let truncated: String = trimmed.chars().take(MAX_FILENAME_LENGTH).collect();
    if truncated.is_empty() {
        "untitled".to_string()
    } else {
        truncated
    }
}

/// Validates a path stored in the database or an import archive: it must be relative and must
/// not escape its base directory.
pub fn is_safe_relative_path(path: &str) -> bool {
    let candidate = Path::new(path);
    if path.is_empty() || candidate.is_absolute() || path.contains('\\') || path.contains(':') {
        return false;
    }
    candidate
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_produces_safe_components() {
        assert_eq!(sanitize_filename("Acme Corp / 2024"), "acme-corp-2024");
        assert_eq!(sanitize_filename("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize_filename("   "), "untitled");
        assert_eq!(sanitize_filename("report.md"), "report.md");
    }

    #[test]
    fn relative_path_validation_rejects_traversal() {
        assert!(is_safe_relative_path("snapshots/ab/abcdef.bin"));
        assert!(!is_safe_relative_path("../outside"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("a/../../b"));
        assert!(!is_safe_relative_path("C:\\windows"));
        assert!(!is_safe_relative_path(""));
    }

    #[test]
    fn data_dir_prefers_cli_override() {
        let input_cli = PathBuf::from("/tmp/cli-data");
        let input_config = PathBuf::from("/tmp/config-data");
        let actual =
            AppPaths::resolve_data_dir(Some(&input_cli), Some(&input_config)).expect("resolves");
        assert_eq!(actual, input_cli);
    }
}
