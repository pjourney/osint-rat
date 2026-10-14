//! Versioned schema migrations. Migrations are append-only: never edit a released migration,
//! add a new one instead.

use rusqlite::{Connection, params};

use crate::model::now;

/// `(version, description, SQL)`.
pub const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "initial schema", V1),
    (
        2,
        "ontology runtime: proposal audit actor and link-type catalog",
        V2,
    ),
    (
        3,
        "entity resolution bags with reversible property and source writes",
        V3,
    ),
    (
        4,
        "playbook runs and watch-set automations (local; not archived)",
        V4,
    ),
    (
        5,
        "first-class event objects with entity links (intervals and instants)",
        V5,
    ),
    (
        6,
        "investigation markings, append-only audit log, and case members",
        V6,
    ),
];

const V1: &str = r#"
CREATE TABLE investigations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX investigations_name ON investigations (name COLLATE NOCASE);

CREATE TABLE entities (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    provenance TEXT NOT NULL,
    fields TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq),
    UNIQUE (investigation_id, kind, value)
);

CREATE TABLE pages (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT,
    description TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    entity_id TEXT REFERENCES entities (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq)
);

CREATE TABLE sources (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    url TEXT,
    retrieved_at TEXT NOT NULL,
    last_seen_at TEXT,
    published_at TEXT,
    author TEXT,
    collector TEXT,
    content_hash TEXT,
    snapshot_path TEXT,
    content TEXT NOT NULL DEFAULT '',
    fields TEXT NOT NULL DEFAULT '[]',
    notes TEXT NOT NULL DEFAULT '',
    provenance TEXT NOT NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    job_id TEXT,
    UNIQUE (investigation_id, seq)
);
CREATE INDEX sources_dedupe ON sources (investigation_id, collector, content_hash);

CREATE TABLE findings (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL,
    provenance TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq)
);

CREATE TABLE finding_sources (
    finding_id TEXT NOT NULL REFERENCES findings (id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    PRIMARY KEY (finding_id, source_id)
);

CREATE TABLE entity_sources (
    entity_id TEXT NOT NULL REFERENCES entities (id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    PRIMARY KEY (entity_id, source_id)
);

CREATE TABLE page_entities (
    page_id TEXT NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    entity_id TEXT NOT NULL REFERENCES entities (id) ON DELETE CASCADE,
    PRIMARY KEY (page_id, entity_id)
);

CREATE TABLE relationships (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    from_id TEXT NOT NULL REFERENCES entities (id) ON DELETE CASCADE,
    to_id TEXT NOT NULL REFERENCES entities (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL,
    confidence TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq),
    UNIQUE (from_id, kind, to_id)
);

CREATE TABLE relationship_sources (
    relationship_id TEXT NOT NULL REFERENCES relationships (id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources (id) ON DELETE CASCADE,
    PRIMARY KEY (relationship_id, source_id)
);

CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    body TEXT NOT NULL,
    provenance TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq)
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    mime TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    sha256 TEXT,
    fields TEXT NOT NULL DEFAULT '[]',
    source_id TEXT REFERENCES sources (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq),
    UNIQUE (investigation_id, path)
);

CREATE TABLE timeline_events (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    occurred_at TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    provenance TEXT NOT NULL,
    source_id TEXT REFERENCES sources (id) ON DELETE SET NULL,
    entity_id TEXT REFERENCES entities (id) ON DELETE SET NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq),
    UNIQUE (investigation_id, kind, occurred_at, title)
);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    investigation_id TEXT REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    program TEXT,
    args TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    progress TEXT,
    log TEXT NOT NULL DEFAULT '',
    stderr TEXT NOT NULL DEFAULT '',
    result TEXT,
    error TEXT,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL
);
CREATE INDEX jobs_investigation ON jobs (investigation_id, seq);

CREATE TABLE agent_runs (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    thread_id TEXT NOT NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    agent_name TEXT NOT NULL,
    program TEXT NOT NULL,
    args TEXT NOT NULL DEFAULT '[]',
    question TEXT NOT NULL,
    response TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    error TEXT,
    focus TEXT,
    context_artifact_id TEXT REFERENCES artifacts (id) ON DELETE SET NULL,
    job_id TEXT,
    model TEXT,
    cost_usd REAL,
    created_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (investigation_id, seq)
);

CREATE TABLE proposals (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    agent_run_id TEXT NOT NULL REFERENCES agent_runs (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    action TEXT NOT NULL,
    status TEXT NOT NULL,
    error TEXT,
    created_at TEXT NOT NULL,
    decided_at TEXT,
    UNIQUE (investigation_id, seq)
);

CREATE TABLE ui_state (
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (scope, key)
);
"#;

/// Action audit actor on proposals, plus first-seen link verbs per investigation.
/// Forward-only: do not edit. Property bags stay on `entities.fields`.
const V2: &str = r#"
ALTER TABLE proposals ADD COLUMN decided_by TEXT;

CREATE TABLE link_types (
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    inverse TEXT,
    first_seen_at TEXT NOT NULL,
    PRIMARY KEY (investigation_id, kind)
);
"#;

/// Resolution bags. Forward-only: do not edit. Constituent entity rows stay in place.
const V3: &str = r#"
CREATE TABLE resolutions (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    canonical_id TEXT NOT NULL REFERENCES entities (id) ON DELETE RESTRICT,
    constituent_ids TEXT NOT NULL,
    status TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL,
    unresolved_by TEXT,
    unresolved_at TEXT,
    property_writes TEXT NOT NULL DEFAULT '[]',
    source_writes TEXT NOT NULL DEFAULT '[]',
    UNIQUE (investigation_id, seq)
);
CREATE INDEX resolutions_investigation ON resolutions (investigation_id, status);
"#;

/// Playbook runs and automations. Forward-only: do not edit. Not exported (like ui_state).
const V4: &str = r#"
CREATE TABLE playbook_runs (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    playbook_id TEXT NOT NULL,
    playbook_name TEXT NOT NULL,
    seed_kind TEXT NOT NULL,
    seed_value TEXT NOT NULL,
    dry_run INTEGER NOT NULL DEFAULT 0,
    automation INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    error TEXT,
    created_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (investigation_id, seq)
);
CREATE INDEX playbook_runs_investigation ON playbook_runs (investigation_id);

CREATE TABLE automations (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    playbook_id TEXT NOT NULL,
    seed_kind TEXT NOT NULL,
    seed_value TEXT NOT NULL,
    interval_seconds INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run_at TEXT,
    last_digest TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq)
);
CREATE INDEX automations_investigation ON automations (investigation_id, playbook_id);
"#;

/// First-class Event objects. Forward-only: do not edit.
const V5: &str = r#"
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL,
    ended_at TEXT,
    provenance TEXT NOT NULL,
    source_id TEXT REFERENCES sources (id) ON DELETE SET NULL,
    page_id TEXT REFERENCES pages (id) ON DELETE SET NULL,
    fields TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, seq),
    UNIQUE (investigation_id, kind, started_at, title)
);
CREATE INDEX events_investigation ON events (investigation_id, started_at);
CREATE TABLE event_entities (
    event_id TEXT NOT NULL REFERENCES events (id) ON DELETE CASCADE,
    entity_id TEXT NOT NULL REFERENCES entities (id) ON DELETE CASCADE,
    PRIMARY KEY (event_id, entity_id)
);
CREATE INDEX event_entities_entity ON event_entities (entity_id);
"#;

/// Markings, append-only audit, members. Forward-only: do not edit.
const V6: &str = r#"
ALTER TABLE investigations ADD COLUMN marking TEXT NOT NULL DEFAULT 'unmarked';

CREATE TABLE audit_log (
    id TEXT PRIMARY KEY,
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    machine TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    summary TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '[]',
    target_ids TEXT NOT NULL DEFAULT '[]',
    UNIQUE (investigation_id, seq)
);
CREATE INDEX audit_log_investigation ON audit_log (investigation_id, seq);
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

CREATE TABLE investigation_members (
    investigation_id TEXT NOT NULL REFERENCES investigations (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    added_by TEXT NOT NULL,
    added_at TEXT NOT NULL,
    PRIMARY KEY (investigation_id, user_id)
);
"#;

/// Applies all pending migrations inside individual transactions. Returns the number applied.
pub fn migrate(connection: &mut Connection) -> rusqlite::Result<usize> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;
    let current = current_version(connection)?;
    let mut applied = 0;
    for (version, description, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, description, applied_at) VALUES (?1, ?2, ?3)",
            params![version, description, now().to_rfc3339()],
        )?;
        transaction.commit()?;
        applied += 1;
    }
    Ok(applied)
}

pub fn current_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )
}

pub fn latest_version() -> i64 {
    MIGRATIONS
        .last()
        .map(|(version, _, _)| *version)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once_and_are_idempotent() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        let first = migrate(&mut connection).expect("first migration");
        let second = migrate(&mut connection).expect("second migration");
        assert_eq!(first, MIGRATIONS.len());
        assert_eq!(second, 0);
        assert_eq!(
            current_version(&connection).expect("version"),
            latest_version()
        );
    }

    #[test]
    fn v2_is_forward_only_and_preserves_v1_rows() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    description TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );",
            )
            .expect("migrations table");
        connection.execute_batch(V1).expect("v1");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, description, applied_at) VALUES (1, 'initial schema', ?1)",
                params![now().to_rfc3339()],
            )
            .expect("record v1");
        connection
            .execute(
                "INSERT INTO investigations (id, name, description, tags, created_at, updated_at)
                 VALUES ('11111111-1111-4111-8111-111111111111', 'V1 Case', '', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("v1 investigation");
        connection
            .execute(
                "INSERT INTO agent_runs (id, investigation_id, seq, thread_id, page_id, agent_name, program, args, question, response, status, error, focus, context_artifact_id, job_id, model, cost_usd, created_at, finished_at)
                 VALUES ('22222222-2222-4222-8222-222222222222', '11111111-1111-4111-8111-111111111111', 1, '33333333-3333-4333-8333-333333333333', NULL, 'test', 'claude', '[]', 'q', '', 'succeeded', NULL, NULL, NULL, NULL, NULL, NULL, ?1, NULL)",
                params![now().to_rfc3339()],
            )
            .expect("v1 run");
        connection
            .execute(
                "INSERT INTO proposals (id, investigation_id, agent_run_id, seq, action, status, error, created_at, decided_at)
                 VALUES ('44444444-4444-4444-8444-444444444444', '11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222', 1, '{\"action\":\"ADD_NOTE\",\"body\":\"hi\"}', 'accepted', NULL, ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("v1 proposal");
        connection
            .execute(
                "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
                 VALUES ('55555555-5555-4555-8555-555555555555', '11111111-1111-4111-8111-111111111111', 1, 'domain', 'example.com', '', '', 'analyst', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("v1 entity");
        connection
            .execute(
                "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
                 VALUES ('66666666-6666-4666-8666-666666666666', '11111111-1111-4111-8111-111111111111', 2, 'ip', '192.0.2.10', '', '', 'collected', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("v1 related entity");
        connection
            .execute(
                "INSERT INTO relationships (id, investigation_id, seq, from_id, to_id, kind, provenance, confidence, notes, created_at)
                 VALUES ('77777777-7777-4777-8777-777777777777', '11111111-1111-4111-8111-111111111111', 1, '55555555-5555-4555-8555-555555555555', '66666666-6666-4666-8666-666666666666', 'resolves_to', 'collected', 'medium', '', ?1)",
                params![now().to_rfc3339()],
            )
            .expect("v1 relationship");
        connection
            .execute(
                "INSERT INTO findings (id, investigation_id, seq, page_id, title, details, confidence, provenance, created_at, updated_at)
                 VALUES ('88888888-8888-4888-8888-888888888888', '11111111-1111-4111-8111-111111111111', 1, NULL, 'Apex domain is live', '', 'medium', 'analyst', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("v1 finding");
        assert_eq!(current_version(&connection).expect("v1 version"), 1);
        let applied = migrate(&mut connection).expect("applies later versions");
        assert_eq!(applied, 5);
        assert_eq!(
            current_version(&connection).expect("latest"),
            latest_version()
        );
        assert_eq!(migrate(&mut connection).expect("idempotent"), 0);
        let decided_by: Option<String> = connection
            .query_row(
                "SELECT decided_by FROM proposals WHERE id = '44444444-4444-4444-8444-444444444444'",
                [],
                |row| row.get(0),
            )
            .expect("survives");
        assert!(decided_by.is_none());
        let entity_value: String = connection
            .query_row(
                "SELECT value FROM entities WHERE id = '55555555-5555-4555-8555-555555555555'",
                [],
                |row| row.get(0),
            )
            .expect("entity survives");
        assert_eq!(entity_value, "example.com");
        let relationship_kind: String = connection
            .query_row(
                "SELECT kind FROM relationships WHERE id = '77777777-7777-4777-8777-777777777777'",
                [],
                |row| row.get(0),
            )
            .expect("relationship survives");
        assert_eq!(relationship_kind, "resolves_to");
        let finding_title: String = connection
            .query_row(
                "SELECT title FROM findings WHERE id = '88888888-8888-4888-8888-888888888888'",
                [],
                |row| row.get(0),
            )
            .expect("finding survives");
        assert_eq!(finding_title, "Apex domain is live");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'link_types'",
                [],
                |row| row.get(0),
            )
            .expect("link_types");
        assert_eq!(count, 1);
        let down: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version > 6",
                [],
                |row| row.get(0),
            )
            .expect("no down migration");
        assert_eq!(down, 0);
        let events_table: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'events'",
                [],
                |row| row.get(0),
            )
            .expect("events");
        assert_eq!(events_table, 1);
        let resolutions: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'resolutions'",
                [],
                |row| row.get(0),
            )
            .expect("resolutions");
        assert_eq!(resolutions, 1);
    }

    #[test]
    fn v3_is_forward_only_and_preserves_v2_rows() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    description TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );",
            )
            .expect("migrations table");
        connection.execute_batch(V1).expect("v1");
        connection.execute_batch(V2).expect("v2");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, description, applied_at) VALUES (1, 'initial schema', ?1), (2, 'ontology runtime', ?1)",
                params![now().to_rfc3339()],
            )
            .expect("record v1+v2");
        connection
            .execute(
                "INSERT INTO investigations (id, name, description, tags, created_at, updated_at)
                 VALUES ('11111111-1111-4111-8111-111111111111', 'V2 Case', '', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("investigation");
        connection
            .execute(
                "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
                 VALUES ('55555555-5555-4555-8555-555555555555', '11111111-1111-4111-8111-111111111111', 1, 'domain', 'example.com', '', '', 'analyst', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("entity");
        assert_eq!(current_version(&connection).expect("v2 version"), 2);
        let applied = migrate(&mut connection).expect("applies v3+");
        assert_eq!(applied, 4);
        assert_eq!(
            current_version(&connection).expect("latest"),
            latest_version()
        );
        assert_eq!(migrate(&mut connection).expect("idempotent"), 0);
        let entity_value: String = connection
            .query_row(
                "SELECT value FROM entities WHERE id = '55555555-5555-4555-8555-555555555555'",
                [],
                |row| row.get(0),
            )
            .expect("entity survives");
        assert_eq!(entity_value, "example.com");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM resolutions", [], |row| row.get(0))
            .expect("empty resolutions");
        assert_eq!(count, 0);
        let playbook_runs: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'playbook_runs'",
                [],
                |row| row.get(0),
            )
            .expect("playbook_runs");
        assert_eq!(playbook_runs, 1);
    }

    #[test]
    fn v4_is_forward_only_and_preserves_v3_rows() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    description TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );",
            )
            .expect("migrations table");
        connection.execute_batch(V1).expect("v1");
        connection.execute_batch(V2).expect("v2");
        connection.execute_batch(V3).expect("v3");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, description, applied_at) VALUES (1, 'initial schema', ?1), (2, 'ontology runtime', ?1), (3, 'resolution bags', ?1)",
                params![now().to_rfc3339()],
            )
            .expect("record v1-v3");
        connection
            .execute(
                "INSERT INTO investigations (id, name, description, tags, created_at, updated_at)
                 VALUES ('11111111-1111-4111-8111-111111111111', 'V3 Case', '', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("investigation");
        connection
            .execute(
                "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
                 VALUES ('55555555-5555-4555-8555-555555555555', '11111111-1111-4111-8111-111111111111', 1, 'domain', 'example.com', '', '', 'analyst', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("entity");
        assert_eq!(current_version(&connection).expect("v3 version"), 3);
        let applied = migrate(&mut connection).expect("applies v4+");
        assert_eq!(applied, 3);
        assert_eq!(
            current_version(&connection).expect("latest"),
            latest_version()
        );
        assert_eq!(migrate(&mut connection).expect("idempotent"), 0);
        let entity_value: String = connection
            .query_row(
                "SELECT value FROM entities WHERE id = '55555555-5555-4555-8555-555555555555'",
                [],
                |row| row.get(0),
            )
            .expect("entity survives");
        assert_eq!(entity_value, "example.com");
        let down: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version > 6",
                [],
                |row| row.get(0),
            )
            .expect("no down migration");
        assert_eq!(down, 0);
    }

    #[test]
    fn v5_is_forward_only_and_preserves_v4_rows() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    description TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );",
            )
            .expect("migrations table");
        connection.execute_batch(V1).expect("v1");
        connection.execute_batch(V2).expect("v2");
        connection.execute_batch(V3).expect("v3");
        connection.execute_batch(V4).expect("v4");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, description, applied_at) VALUES (1, 'initial schema', ?1), (2, 'ontology runtime', ?1), (3, 'resolution bags', ?1), (4, 'playbooks', ?1)",
                params![now().to_rfc3339()],
            )
            .expect("record v1-v4");
        connection
            .execute(
                "INSERT INTO investigations (id, name, description, tags, created_at, updated_at)
                 VALUES ('11111111-1111-4111-8111-111111111111', 'V4 Case', '', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("investigation");
        connection
            .execute(
                "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
                 VALUES ('55555555-5555-4555-8555-555555555555', '11111111-1111-4111-8111-111111111111', 1, 'ip', '192.0.2.1', '', '', 'collected', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("entity");
        assert_eq!(current_version(&connection).expect("v4 version"), 4);
        let applied = migrate(&mut connection).expect("applies v5+");
        assert_eq!(applied, 2);
        assert_eq!(
            current_version(&connection).expect("latest"),
            latest_version()
        );
        assert_eq!(migrate(&mut connection).expect("idempotent"), 0);
        let entity_value: String = connection
            .query_row(
                "SELECT value FROM entities WHERE id = '55555555-5555-4555-8555-555555555555'",
                [],
                |row| row.get(0),
            )
            .expect("entity survives");
        assert_eq!(entity_value, "192.0.2.1");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .expect("empty events");
        assert_eq!(count, 0);
        let down: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version > 6",
                [],
                |row| row.get(0),
            )
            .expect("no down migration");
        assert_eq!(down, 0);
    }

    #[test]
    fn v6_is_forward_only_and_preserves_v5_rows() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    description TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );",
            )
            .expect("migrations table");
        connection.execute_batch(V1).expect("v1");
        connection.execute_batch(V2).expect("v2");
        connection.execute_batch(V3).expect("v3");
        connection.execute_batch(V4).expect("v4");
        connection.execute_batch(V5).expect("v5");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, description, applied_at) VALUES (1, 'initial schema', ?1), (2, 'ontology runtime', ?1), (3, 'resolution bags', ?1), (4, 'playbooks', ?1), (5, 'events', ?1)",
                params![now().to_rfc3339()],
            )
            .expect("record v1-v5");
        connection
            .execute(
                "INSERT INTO investigations (id, name, description, tags, created_at, updated_at)
                 VALUES ('11111111-1111-4111-8111-111111111111', 'V5 Case', '', '[]', ?1, ?1)",
                params![now().to_rfc3339()],
            )
            .expect("investigation");
        assert_eq!(current_version(&connection).expect("v5 version"), 5);
        let applied = migrate(&mut connection).expect("applies v6");
        assert_eq!(applied, 1);
        assert_eq!(current_version(&connection).expect("v6 version"), 6);
        assert_eq!(migrate(&mut connection).expect("idempotent"), 0);
        let marking: String = connection
            .query_row(
                "SELECT marking FROM investigations WHERE id = '11111111-1111-4111-8111-111111111111'",
                [],
                |row| row.get(0),
            )
            .expect("marking default");
        assert_eq!(marking, "unmarked");
        let name: String = connection
            .query_row(
                "SELECT name FROM investigations WHERE id = '11111111-1111-4111-8111-111111111111'",
                [],
                |row| row.get(0),
            )
            .expect("survives");
        assert_eq!(name, "V5 Case");
        let audit: i64 = connection
            .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))
            .expect("empty audit");
        assert_eq!(audit, 0);
    }

    #[test]
    fn schema_contains_core_tables() {
        let mut connection = Connection::open_in_memory().expect("in-memory db");
        migrate(&mut connection).expect("migrates");
        let expected = [
            "investigations",
            "pages",
            "sources",
            "findings",
            "entities",
            "relationships",
            "link_types",
            "jobs",
            "agent_runs",
            "resolutions",
            "playbook_runs",
            "automations",
            "events",
            "event_entities",
            "audit_log",
            "investigation_members",
        ];
        for table in expected {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("query");
            assert_eq!(count, 1, "table {table} exists");
        }
    }
}
