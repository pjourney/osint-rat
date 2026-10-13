//! Command palette entries.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub hint: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command {
        id: "new_investigation",
        title: "New Investigation",
        hint: "n",
    },
    Command {
        id: "open_investigation",
        title: "Open Investigation",
        hint: "o",
    },
    Command {
        id: "new_page",
        title: "New Page",
        hint: "n",
    },
    Command {
        id: "run_collectors",
        title: "Run Collectors",
        hint: "r",
    },
    Command {
        id: "run_dns",
        title: "Run DNS Collection",
        hint: "",
    },
    Command {
        id: "run_rdap",
        title: "Run RDAP Collection",
        hint: "",
    },
    Command {
        id: "run_http",
        title: "Fetch URL / HTTP",
        hint: "",
    },
    Command {
        id: "run_tls",
        title: "Run TLS Collection",
        hint: "",
    },
    Command {
        id: "run_ip",
        title: "Run IP Intelligence",
        hint: "",
    },
    Command {
        id: "ask_agent",
        title: "Ask Agent",
        hint: "a",
    },
    Command {
        id: "generate_report",
        title: "Generate Report",
        hint: "",
    },
    Command {
        id: "add_finding",
        title: "Add Finding",
        hint: "",
    },
    Command {
        id: "add_note",
        title: "Add Note",
        hint: "",
    },
    Command {
        id: "open_entities",
        title: "Open Entities",
        hint: "",
    },
    Command {
        id: "open_sources",
        title: "Open Sources",
        hint: "",
    },
    Command {
        id: "open_findings",
        title: "Open Findings",
        hint: "",
    },
    Command {
        id: "open_timeline",
        title: "Open Timeline",
        hint: "",
    },
    Command {
        id: "open_jobs",
        title: "Show Jobs",
        hint: "",
    },
    Command {
        id: "open_tools",
        title: "Show Tools",
        hint: "",
    },
    Command {
        id: "open_agent",
        title: "Agent History",
        hint: "",
    },
    Command {
        id: "open_settings",
        title: "Configure Agent",
        hint: "",
    },
    Command {
        id: "open_artifacts",
        title: "Open Artifacts",
        hint: "",
    },
    Command {
        id: "doctor",
        title: "Doctor",
        hint: "d",
    },
    Command {
        id: "help",
        title: "Help",
        hint: "?",
    },
    Command {
        id: "cancel_job",
        title: "Cancel Running Job",
        hint: "c",
    },
];

pub fn filtered(query: &str) -> Vec<&'static Command> {
    let query = query.trim().to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|command| {
            query.is_empty()
                || command.title.to_ascii_lowercase().contains(&query)
                || command.id.contains(&query)
        })
        .collect()
}
