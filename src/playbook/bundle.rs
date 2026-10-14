//! Bundled playbooks compiled into the binary.

use crate::model::{Playbook, PlaybookOrigin};

use super::parse::parse_playbook_yaml;

const DOMAIN_INFRA: &str = include_str!("../../playbooks/domain-infra.yaml");
const MAIL_POLICY: &str = include_str!("../../playbooks/mail-policy.yaml");
const CERT_ESTATE: &str = include_str!("../../playbooks/cert-estate.yaml");
const REPO_SURFACE: &str = include_str!("../../playbooks/repo-surface.yaml");

/// Bundled documents in display order.
pub const BUNDLED: &[(&str, &str)] = &[
    ("domain-infra", DOMAIN_INFRA),
    ("mail-policy", MAIL_POLICY),
    ("cert-estate", CERT_ESTATE),
    ("repo-surface", REPO_SURFACE),
];

pub fn bundled_ids() -> Vec<&'static str> {
    BUNDLED.iter().map(|(id, _)| *id).collect()
}

pub fn bundled(id: &str) -> Option<Playbook> {
    let yaml = BUNDLED
        .iter()
        .find(|(candidate, _)| *candidate == id.trim())
        .map(|(_, body)| *body)?;
    let mut playbook = parse_playbook_yaml(yaml.as_bytes()).ok()?;
    playbook.origin = PlaybookOrigin::Bundled;
    Some(playbook)
}

/// Fixed agent question for a bundled (or any) playbook. Suggested Actions are listed;
/// the agent still only proposes.
pub fn bundled_prompt(playbook: &Playbook, seed: &str) -> String {
    let mut lines = vec![
        format!(
            "Review this investigation using the {} playbook (`{}`). Seed: {} {}.",
            playbook.name,
            playbook.id,
            playbook.seed.kind.as_str(),
            seed.trim()
        ),
        "Propose Actions only. Do not claim collectors already ran. Do not invent collected facts."
            .into(),
        "Do not propose active tools, exploits, or silent writes. Suggested Actions:".into(),
    ];
    for step in &playbook.steps {
        match step {
            crate::model::PlaybookStep::Collect { collector } => {
                lines.push(format!(
                    "- RUN_COLLECTOR collector={collector} target={}",
                    seed.trim()
                ));
            }
            crate::model::PlaybookStep::RunTool { tool } => {
                lines.push(format!(
                    "- (passive tool {tool} if available; never an active probe)"
                ));
            }
            crate::model::PlaybookStep::SearchAround { hops, .. } => {
                lines.push(format!("- Search Around is a graph query ({hops} hops); do not propose RUN_PLAYBOOK recursively"));
            }
            crate::model::PlaybookStep::AssertFindingTemplate { title, details } => {
                lines.push(format!(
                    "- ASSERT_FINDING_TEMPLATE title={title:?} details={details:?}"
                ));
            }
            crate::model::PlaybookStep::ProposeResolve { reason } => {
                lines.push(format!(
                    "- PROPOSE_RESOLVE if duplicates exist (reason: {reason}). Never auto-merge."
                ));
            }
        }
    }
    lines.push(
        "You may also propose RUN_PLAYBOOK with this bundled id after the analyst confirms.".into(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_playbook_parses() {
        for (id, yaml) in BUNDLED {
            let actual = parse_playbook_yaml(yaml.as_bytes()).expect(id);
            assert_eq!(actual.id, *id);
            assert!(!actual.steps.is_empty(), "{id} needs steps");
        }
    }
}
