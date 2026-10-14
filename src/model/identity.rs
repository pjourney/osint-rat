//! Local actor identity recorded on audit rows and Action decisions.

use std::env;
use std::fs;

/// Who is operating this os-int process. Configurable; never a network identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub user_id: String,
    pub machine_name: String,
}

impl Identity {
    /// `$USER` / `$USERNAME` and the local hostname, else `local` / `unknown`.
    pub fn from_env() -> Self {
        Self {
            user_id: env_user(),
            machine_name: hostname(),
        }
    }

    /// Config values win when non-empty; otherwise [`Identity::from_env`].
    pub fn resolve(user_id: &str, machine_name: &str) -> Self {
        let fallback = Self::from_env();
        Self {
            user_id: nonempty(user_id).unwrap_or(fallback.user_id),
            machine_name: nonempty(machine_name).unwrap_or(fallback.machine_name),
        }
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::from_env()
    }
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_user() -> String {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .ok()
        .and_then(|value| nonempty(&value))
        .unwrap_or_else(|| "local".into())
}

fn hostname() -> String {
    #[cfg(unix)]
    {
        if let Some(name) = unix_hostname() {
            return name;
        }
    }
    env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .ok()
        .and_then(|value| nonempty(&value))
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .and_then(|value| nonempty(&value))
        })
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(unix)]
fn unix_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is a valid writable array; gethostname writes a NUL-terminated name or fails.
    let result = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if result != 0 {
        return None;
    }
    let len = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..len]).ok().and_then(nonempty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_explicit_values() {
        let actual = Identity::resolve(" analyst ", " laptop.local ");
        assert_eq!(actual.user_id, "analyst");
        assert_eq!(actual.machine_name, "laptop.local");
    }

    #[test]
    fn resolve_falls_back_when_blank() {
        let actual = Identity::resolve("  ", "");
        assert!(!actual.user_id.trim().is_empty());
        assert!(!actual.machine_name.trim().is_empty());
    }
}
