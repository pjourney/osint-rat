//! os-int: an open-source intelligence investigation workbench for the terminal.
//!
//! An investigation is a case. Pages are the subjects you are working. Collectors and
//! external tools turn public records into sources, entities, relationships, findings, and
//! timeline events. A local agent CLI can read that case and propose changes; nothing it
//! proposes is written until the analyst accepts it.
//!
//! # Where to read next
//!
//! * The user guide, examples, FAQ, and table map live in `docs/`.
//! * [`docs/ARCHITECTURE.md`](https://github.com/pjourney/osint-rat/blob/main/docs/ARCHITECTURE.md)
//!   records the layering and the decisions behind it.
//!
//! # Modules
//!
//! | Module | Role |
//! | --- | --- |
//! | [`model`] | Domain types. No terminal and no SQLite. |
//! | [`storage`] | SQLite. Applies a collector batch in one transaction. |
//! | [`files`] | Content-addressed snapshots, reports, and agent context packs. |
//! | [`collectors`] | Passive collectors. They return a batch and do not write the database. |
//! | [`tools`] | External programs: built-ins plus `[[tools.custom]]`. |
//! | [`exec`] | The only place a subprocess is spawned. No shell. |
//! | [`agent`] | Local agent CLIs, context fences, and proposal parsing. |
//! | [`jobs`] | Background work and the channel back to the UI. |
//! | [`app`] | State and [`app::Action`]s. Testable without a terminal. |
//! | [`tui`] | Event loop and rendering. Drawing is a function of [`app::App`]. |
//! | [`cli`] | Clap commands. No subcommand launches the workbench. |
//! | [`report`] | Markdown and JSON reports. |
//! | [`exchange`] | Versioned import and export. |
//! | [`ingest`] | STIX, MISP, CSV/JSON, and Evidence Batch file ingest. |
//! | [`config`] | `config.toml`. Missing keys use the defaults in this module. |
//! | [`paths`] | Platform config and data directories. |
//! | [`sanitize`] | Strips terminal escapes from untrusted text. |
//! | [`doctor`] | Environment checks for `os-int doctor` and the welcome screen. |
//! | [`logging`] | Daily log files. Log lines never go to the workbench. |

pub mod agent;
pub mod app;
pub mod cli;
pub mod collectors;
pub mod config;
pub mod doctor;
pub mod exchange;
pub mod exec;
pub mod files;
pub mod ingest;
pub mod jobs;
pub mod logging;
pub mod model;
pub mod paths;
pub mod report;
pub mod sanitize;
pub mod storage;
pub mod tools;
pub mod tui;
