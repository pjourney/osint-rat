//! CLI production-readiness: `--json` gaps, write gates, error paths, import/export.

use std::path::{Path, PathBuf};
use std::process::Command;

use os_int::paths::AppPaths;
use os_int::storage::Store;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_os-int"))
}

fn sandbox() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().expect("tempdir");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let paths = AppPaths::new(config.clone(), data.clone());
    paths.ensure_dirs().expect("dirs");
    (directory, config, data)
}

fn run(config: &Path, data: &Path, args: &[&str]) -> std::process::Output {
    bin()
        .args([
            "--config",
            config.to_str().expect("utf-8"),
            "--data-dir",
            data.to_str().expect("utf-8"),
        ])
        .args(args)
        .output()
        .expect("os-int")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn seed_case(config: &Path, data: &Path) -> String {
    let paths = AppPaths::new(config.to_path_buf(), data.to_path_buf());
    paths.ensure_dirs().expect("dirs");
    let store = Store::open(&paths.database()).expect("store");
    store.create_investigation("Case", "").expect("case").name
}

fn import_demo(config: &Path, data: &Path) {
    let archive = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo-case");
    let output = run(config, data, &["import", archive.to_str().expect("utf-8")]);
    assert!(
        output.status.success(),
        "import demo: {}\n{}",
        stdout(&output),
        stderr(&output)
    );
}

#[test]
fn playbook_list_json_emits_bundled_ids() {
    let (_dir, config, data) = sandbox();
    let output = run(&config, &data, &["--json", "playbook", "list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    let ids: Vec<&str> = value
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|row| row["id"].as_str())
        .collect();
    assert!(ids.contains(&"domain-infra"), "{ids:?}");
    assert!(ids.contains(&"mail-policy"), "{ids:?}");
}

#[test]
fn doctor_json_is_an_array_of_checks() {
    let (_dir, config, data) = sandbox();
    let output = run(&config, &data, &["--json", "doctor"]);
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    let checks = value.as_array().expect("array");
    assert!(
        checks.iter().any(|check| check["name"] == "Database"),
        "{value}"
    );
    assert!(
        checks.iter().any(|check| check["status"].is_string()),
        "{value}"
    );
}

#[test]
fn team_list_and_marking_get_and_cooccur_json() {
    let (_dir, config, data) = sandbox();
    import_demo(&config, &data);
    let team = run(&config, &data, &["--json", "team", "list", "Example Corp"]);
    assert!(team.status.success(), "{}", stderr(&team));
    let team_json: serde_json::Value = serde_json::from_str(&stdout(&team)).expect("team");
    assert_eq!(team_json["investigation"], "Example Corp");
    assert!(team_json["members"].is_array(), "{team_json}");
    let marking = run(
        &config,
        &data,
        &["--json", "marking", "get", "Example Corp"],
    );
    assert!(marking.status.success(), "{}", stderr(&marking));
    let marking_json: serde_json::Value = serde_json::from_str(&stdout(&marking)).expect("marking");
    assert_eq!(marking_json["investigation"], "Example Corp");
    assert!(marking_json["marking"].is_string(), "{marking_json}");
    let cooccur = run(&config, &data, &["--json", "cooccur", "Example Corp"]);
    assert!(cooccur.status.success(), "{}", stderr(&cooccur));
    let pairs: serde_json::Value = serde_json::from_str(&stdout(&cooccur)).expect("cooccur");
    assert!(pairs.is_array(), "{pairs}");
}

#[test]
fn plain_case_findings_glance_does_not_use_hollow_diamond() {
    let (_dir, config, data) = sandbox();
    import_demo(&config, &data);
    let output = run(&config, &data, &["--plain", "case", "Example Corp"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("AT A GLANCE"), "{text}");
    assert!(text.contains("findings"), "{text}");
    assert!(!text.contains('◇'), "{text}");
}

#[test]
fn review_accept_without_yes_is_refused_on_a_pipe() {
    let (_dir, config, data) = sandbox();
    seed_case(&config, &data);
    let output = run(&config, &data, &["review", "accept", "Case", "X-1"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains("--yes") || err.contains("no Review item"),
        "{err}"
    );
}

#[test]
fn unknown_investigation_and_unknown_kind_are_clean_errors() {
    let (_dir, config, data) = sandbox();
    seed_case(&config, &data);
    let missing = run(&config, &data, &["case", "No Such Case"]);
    assert!(!missing.status.success());
    let err = stderr(&missing);
    assert!(!err.contains("Location"), "{err}");
    assert!(
        err.contains("No Such Case") || err.contains("no investigation"),
        "{err}"
    );
    let kind = run(
        &config,
        &data,
        &[
            "collect",
            "not-a-kind",
            "example.com",
            "--investigation",
            "Case",
        ],
    );
    assert!(!kind.status.success());
    let kind_err = stderr(&kind);
    assert!(
        kind_err.contains("unknown target kind") || kind_err.contains("not-a-kind"),
        "{kind_err}"
    );
}

#[test]
fn malformed_config_is_a_user_error() {
    let directory = tempfile::tempdir().expect("tempdir");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    std::fs::write(&config, "ui.theme = [unterminated\n").expect("write");
    let output = run(&config, &data, &["list"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains("invalid config") || err.contains("TOML") || err.contains("parse"),
        "{err}"
    );
}

#[test]
fn export_import_round_trip_via_cli() {
    let (_dir, config, data) = sandbox();
    import_demo(&config, &data);
    let export_dir = data.join("round-trip");
    let export = run(
        &config,
        &data,
        &[
            "export",
            "Example Corp",
            "-o",
            export_dir.to_str().expect("utf-8"),
        ],
    );
    assert!(export.status.success(), "{}", stderr(&export));
    let (_dir2, config2, data2) = sandbox();
    let imported = run(
        &config2,
        &data2,
        &["import", export_dir.to_str().expect("utf-8")],
    );
    assert!(imported.status.success(), "{}", stderr(&imported));
    let list = run(&config2, &data2, &["--json", "list"]);
    assert!(list.status.success(), "{}", stderr(&list));
    let value: serde_json::Value = serde_json::from_str(&stdout(&list)).expect("list");
    let names: Vec<&str> = value
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|row| row["investigation"]["name"].as_str())
        .collect();
    assert!(
        names.iter().any(|name| name.contains("Example Corp")),
        "{value}"
    );
}

#[test]
fn ingest_malformed_file_is_refused() {
    let (_dir, config, data) = sandbox();
    seed_case(&config, &data);
    let junk = data.join("junk.json");
    std::fs::write(&junk, "{not json").expect("write");
    let output = run(
        &config,
        &data,
        &[
            "ingest",
            junk.to_str().expect("utf-8"),
            "--format",
            "json",
            "--investigation",
            "Case",
        ],
    );
    assert!(!output.status.success());
    assert!(!stderr(&output).contains("Location"));
}

#[test]
fn list_json_empty_workspace_is_an_array() {
    let (_dir, config, data) = sandbox();
    let output = run(&config, &data, &["--json", "list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(value, serde_json::json!([]));
}
