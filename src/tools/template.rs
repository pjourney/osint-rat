//! Argument templates for external tools.
//!
//! Placeholders are replaced *inside* individual arguments; arguments are never re-split, so a
//! target value can never become an extra argument, option, or shell syntax.

use std::fmt;

use crate::model::Target;

pub const PLACEHOLDERS: [&str; 10] = [
    "target", "domain", "host", "ip", "url", "email", "username", "repo", "file", "asn",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    UnknownPlaceholder(String),
    NotApplicable { placeholder: String, target: String },
    LooksLikeOption(String),
    Unterminated(String),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::UnknownPlaceholder(name) => write!(
                formatter,
                "unknown placeholder {{{name}}} (known: {})",
                PLACEHOLDERS.map(|p| format!("{{{p}}}")).join(", ")
            ),
            TemplateError::NotApplicable {
                placeholder,
                target,
            } => {
                write!(
                    formatter,
                    "{{{placeholder}}} cannot be derived from {target}"
                )
            }
            TemplateError::LooksLikeOption(value) => {
                write!(
                    formatter,
                    "refusing to pass {value:?}: values may not start with '-'"
                )
            }
            TemplateError::Unterminated(arg) => {
                write!(formatter, "unterminated placeholder in {arg:?}")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

/// The value a placeholder takes for a target, if it can be derived.
pub fn placeholder_value(name: &str, target: &Target) -> Option<String> {
    match (name, target) {
        ("target", _) => Some(target.value()),
        ("domain", Target::Domain(domain)) => Some(domain.clone()),
        ("domain", Target::Email(_)) | ("domain", Target::Url(_)) => target
            .hostname()
            .filter(|host| host.parse::<std::net::IpAddr>().is_err()),
        ("host", Target::Ip(ip)) => Some(ip.to_string()),
        ("host", _) => target.hostname(),
        ("ip", Target::Ip(ip)) => Some(ip.to_string()),
        ("url", Target::Url(url)) => Some(url.to_string()),
        ("url", Target::Domain(domain)) => Some(format!("https://{domain}/")),
        ("url", Target::Repository(repo)) if repo.starts_with("https://") => Some(repo.clone()),
        ("email", Target::Email(email)) => Some(email.clone()),
        ("username", Target::Username(name)) => Some(name.clone()),
        ("repo", Target::Repository(repo)) => Some(repo.clone()),
        ("file", Target::File(path)) => Some(
            std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.clone())
                .display()
                .to_string(),
        ),
        ("asn", Target::Asn(number)) => Some(format!("AS{number}")),
        _ => None,
    }
}

/// Lists the placeholders used by an argument, validating syntax.
pub fn placeholders_in(arg: &str) -> Result<Vec<String>, TemplateError> {
    let mut found = Vec::new();
    let mut rest = arg;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| TemplateError::Unterminated(arg.to_string()))?;
        let name = &after[..end];
        if !PLACEHOLDERS.contains(&name) {
            return Err(TemplateError::UnknownPlaceholder(name.to_string()));
        }
        found.push(name.to_string());
        rest = &after[end + 1..];
    }
    Ok(found)
}

/// Validates a template without a target (used when loading configuration).
pub fn validate_template(args: &[String]) -> Result<(), TemplateError> {
    for arg in args {
        placeholders_in(arg)?;
    }
    Ok(())
}

/// Substitutes placeholders for `target` in every argument.
pub fn render_args(template: &[String], target: &Target) -> Result<Vec<String>, TemplateError> {
    template.iter().map(|arg| render_arg(arg, target)).collect()
}

fn render_arg(arg: &str, target: &Target) -> Result<String, TemplateError> {
    let mut output = arg.to_string();
    for name in placeholders_in(arg)? {
        let value =
            placeholder_value(&name, target).ok_or_else(|| TemplateError::NotApplicable {
                placeholder: name.clone(),
                target: target.to_string(),
            })?;
        if value.starts_with('-') {
            return Err(TemplateError::LooksLikeOption(value));
        }
        output = output.replace(&format!("{{{name}}}"), &value);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetKind;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn substitutes_inside_arguments_without_splitting() {
        let input_target = Target::parse(TargetKind::Domain, "example.com").expect("domain");
        let input_template = strings(&["-connect", "{host}:443", "-servername", "{host}"]);
        let actual = render_args(&input_template, &input_target).expect("renders");
        assert_eq!(
            actual,
            strings(&["-connect", "example.com:443", "-servername", "example.com"])
        );
    }

    #[test]
    fn derives_urls_and_domains() {
        let input_domain = Target::parse(TargetKind::Domain, "example.com").expect("domain");
        let input_email = Target::parse(TargetKind::Email, "a@example.org").expect("email");
        assert_eq!(
            placeholder_value("url", &input_domain),
            Some("https://example.com/".into())
        );
        assert_eq!(
            placeholder_value("domain", &input_email),
            Some("example.org".into())
        );
        assert_eq!(placeholder_value("ip", &input_domain), None);
    }

    #[test]
    fn rejects_inapplicable_and_unknown_placeholders() {
        let input_target = Target::parse(TargetKind::Domain, "example.com").expect("domain");
        let not_applicable = render_args(&strings(&["{ip}"]), &input_target);
        let unknown = validate_template(&strings(&["{nope}"]));
        let unterminated = validate_template(&strings(&["{target"]));
        assert!(matches!(
            not_applicable,
            Err(TemplateError::NotApplicable { .. })
        ));
        assert!(matches!(unknown, Err(TemplateError::UnknownPlaceholder(_))));
        assert!(matches!(unterminated, Err(TemplateError::Unterminated(_))));
    }

    #[test]
    fn hostile_values_stay_one_argument() {
        let input_target = Target::Username("a;b $(c)".into());
        let actual = render_args(&strings(&["--user={username}"]), &input_target).expect("renders");
        assert_eq!(actual, strings(&["--user=a;b $(c)"]));
    }

    #[test]
    fn option_like_values_are_refused() {
        let input_target = Target::Username("-rf".into());
        let actual = render_args(&strings(&["{username}"]), &input_target);
        assert!(matches!(actual, Err(TemplateError::LooksLikeOption(_))));
    }
}
