//! Redaction of secrets in command lines before they are displayed, logged, or stored.

const REDACTED: &str = "[REDACTED]";
const SENSITIVE_MARKERS: [&str; 9] = [
    "key",
    "token",
    "secret",
    "password",
    "passwd",
    "auth",
    "credential",
    "bearer",
    "cookie",
];
/// Arguments longer than this are elided in displays (they are usually prompt text).
pub const DISPLAY_ARG_LIMIT: usize = 240;

fn is_sensitive_name(name: &str) -> bool {
    let lower = name.trim_start_matches('-').to_ascii_lowercase();
    SENSITIVE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Redacts flag values, `KEY=VALUE` pairs, sensitive HTTP headers, and URL passwords.
pub fn redact_args(args: &[String]) -> Vec<String> {
    let mut output = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            output.push(REDACTED.to_string());
            redact_next = false;
            continue;
        }
        if arg.starts_with('-') && !arg.contains('=') && is_sensitive_name(arg) {
            output.push(arg.clone());
            redact_next = true;
            continue;
        }
        output.push(redact_value(arg));
    }
    output
}

/// Redacts a single value that may embed a secret.
pub fn redact_value(arg: &str) -> String {
    if let Some((name, _)) = arg.split_once('=')
        && !name.contains(' ')
        && is_sensitive_name(name)
    {
        return format!("{name}={REDACTED}");
    }
    if let Some((header, _)) = arg.split_once(':')
        && !header.contains(' ')
        && !header.contains('/')
        && (header.eq_ignore_ascii_case("authorization") || is_sensitive_name(header))
    {
        return format!("{header}: {REDACTED}");
    }
    redact_url_password(arg)
}

fn redact_url_password(arg: &str) -> String {
    let Some(scheme_end) = arg.find("://") else {
        return arg.to_string();
    };
    let rest = &arg[scheme_end + 3..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some((credentials, host)) = authority.rsplit_once('@') else {
        return arg.to_string();
    };
    let user = credentials.split(':').next().unwrap_or_default();
    format!(
        "{}{user}:{REDACTED}@{host}{}",
        &arg[..scheme_end + 3],
        &rest[authority_end..]
    )
}

/// Redacts and shortens arguments for display, eliding very long values such as prompts.
pub fn display_args(args: &[String]) -> Vec<String> {
    redact_args(args)
        .into_iter()
        .map(|arg| {
            if arg.len() > DISPLAY_ARG_LIMIT {
                format!("<{} omitted>", crate::sanitize::format_bytes(arg.len()))
            } else {
                crate::sanitize::sanitize_line(&arg).into_owned()
            }
        })
        .collect()
}

/// A shell-quoted, redacted rendering of a command for transparency displays.
pub fn display_command(program: &str, args: &[String]) -> String {
    let mut parts = vec![quote(program)];
    parts.extend(display_args(args).iter().map(|arg| quote(arg)));
    parts.join(" ")
}

fn quote(value: &str) -> String {
    if value.starts_with('<') && value.ends_with(" omitted>") {
        return value.to_string();
    }
    shlex::try_quote(value)
        .map(|quoted| quoted.into_owned())
        .unwrap_or_else(|_| format!("{value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn redacts_values_after_sensitive_flags() {
        let input = strings(&["--api-key", "sk-123", "--model", "sonnet"]);
        let actual = redact_args(&input);
        assert_eq!(
            actual,
            strings(&["--api-key", "[REDACTED]", "--model", "sonnet"])
        );
    }

    #[test]
    fn redacts_inline_flag_values_and_env_pairs() {
        let input = strings(&["--token=abc", "OPENAI_API_KEY=sk-1", "PATH=/usr/bin"]);
        let actual = redact_args(&input);
        assert_eq!(
            actual,
            strings(&[
                "--token=[REDACTED]",
                "OPENAI_API_KEY=[REDACTED]",
                "PATH=/usr/bin"
            ])
        );
    }

    #[test]
    fn redacts_authorization_headers_and_url_passwords() {
        assert_eq!(
            redact_value("Authorization: Bearer abc"),
            "Authorization: [REDACTED]"
        );
        assert_eq!(redact_value("X-Api-Key: abc"), "X-Api-Key: [REDACTED]");
        assert_eq!(
            redact_value("https://user:hunter2@example.com/path"),
            "https://user:[REDACTED]@example.com/path"
        );
        assert_eq!(
            redact_value("https://example.com/a:b"),
            "https://example.com/a:b"
        );
    }

    #[test]
    fn keeps_harmless_claude_flags() {
        let input = strings(&[
            "-p",
            "--output-format",
            "stream-json",
            "--tools",
            "",
            "--no-session-persistence",
        ]);
        assert_eq!(redact_args(&input), input);
    }

    #[test]
    fn display_elides_long_prompts_and_quotes() {
        let long_prompt = "x".repeat(DISPLAY_ARG_LIMIT + 1);
        let input = strings(&["-p", &long_prompt, "--tools", ""]);
        let actual = display_command("claude", &input);
        assert_eq!(actual, "claude -p <241 B omitted> --tools ''");
    }
}
