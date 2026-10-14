//! Environment checks used by `os-int doctor` and the welcome screen.

use crate::config::Config;
use crate::paths::AppPaths;
use crate::storage::Store;
use crate::tools::ToolRegistry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Missing,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

pub fn run(
    config: &Config,
    paths: &AppPaths,
    store: Option<&Store>,
    tools: &ToolRegistry,
) -> Vec<Check> {
    let mut checks = Vec::new();
    match store {
        Some(store) => match database_health(store) {
            Ok(detail) => checks.push(ok(
                "Database",
                format!(
                    "{} (schema {}) {detail}",
                    paths.database().display(),
                    store.schema_version().unwrap_or(0)
                ),
            )),
            Err(error) => checks.push(warn("Database", error)),
        },
        None => checks.push(Check {
            name: "Database".into(),
            status: CheckStatus::Warn,
            detail: "not opened".into(),
        }),
    }
    checks.push(ok("Config", paths.config_file.display().to_string()));
    for program in [
        "claude", "codex", "dig", "whois", "git", "gh", "exiftool", "curl", "openssl",
    ] {
        let installed = tools.is_installed(program);
        let label = match program {
            "claude" => "Claude Code",
            "codex" => "Codex",
            other => other,
        };
        checks.push(Check {
            name: label.into(),
            status: if installed {
                CheckStatus::Ok
            } else {
                CheckStatus::Missing
            },
            detail: if installed {
                tools
                    .path_of(program)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "installed".into())
            } else {
                "not found on PATH".into()
            },
        });
    }
    let network = if config
        .network
        .proxy
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_some()
    {
        format!(
            "proxy {}",
            config.network.proxy.as_deref().unwrap_or_default()
        )
    } else {
        "direct (timeout {}s)".replace("{}", &config.network.request_timeout_seconds.to_string())
    };
    checks.push(ok("Network", network));
    checks.push(ok("Logs", paths.log_dir().display().to_string()));
    checks
}

fn database_health(store: &Store) -> Result<String, String> {
    let integrity = store.integrity_check().map_err(|error| error.to_string())?;
    if integrity != "ok" {
        return Err(integrity);
    }
    let foreign_keys = store
        .foreign_key_violations()
        .map_err(|error| error.to_string())?;
    if !foreign_keys.is_empty() {
        return Err(format!(
            "foreign key violations: {}",
            foreign_keys.join("; ")
        ));
    }
    Ok("integrity ok, foreign keys ok".into())
}

fn ok(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status: CheckStatus::Ok,
        detail: detail.into(),
    }
}

fn warn(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status: CheckStatus::Warn,
        detail: detail.into(),
    }
}

impl CheckStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            CheckStatus::Ok => "OK",
            CheckStatus::Missing => "missing",
            CheckStatus::Warn => "warn",
        }
    }
}
