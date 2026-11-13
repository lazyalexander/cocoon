//! SQLite metadata ownership and schema migration. Only the host opens this store.
use crate::Result;
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn open(root: &Path) -> Result<Connection> {
    let db = Connection::open(root.join("state.sqlite3"))?;
    let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > 3 {
        return Err("unsupported database schema version".into());
    }
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS images(id TEXT PRIMARY KEY, manifest TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS vms(id TEXT PRIMARY KEY, record TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY, kind TEXT NOT NULL, target TEXT NOT NULL, status TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS boots(id TEXT PRIMARY KEY, vm TEXT NOT NULL, status TEXT NOT NULL, report TEXT);
        CREATE TABLE IF NOT EXISTS journal(sequence INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, vm TEXT NOT NULL, generation INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL, previous_sha256 TEXT NOT NULL, sha256 TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS executions(attempt TEXT PRIMARY KEY, vm TEXT NOT NULL, record TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS rollback_intents(vm TEXT PRIMARY KEY, image TEXT NOT NULL, disk_copy TEXT);
        CREATE TABLE IF NOT EXISTS work_grants(id TEXT PRIMARY KEY, token_sha256 TEXT NOT NULL UNIQUE, parent TEXT, subject TEXT NOT NULL, vm TEXT NOT NULL, generation INTEGER NOT NULL, actions TEXT NOT NULL, expires_unix INTEGER NOT NULL, max_export_bytes INTEGER NOT NULL, revoked INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS consumer_operations(id TEXT PRIMARY KEY, parameter_sha256 TEXT NOT NULL, status TEXT NOT NULL, result TEXT, vm TEXT, attempt TEXT, generation INTEGER, grant_id TEXT);
        PRAGMA user_version=3; COMMIT;")?;
    db.execute("UPDATE images SET manifest=json_set(manifest,'$.approved',json('true')) WHERE json_extract(manifest,'$.approved') IS NULL", [])?;
    Ok(db)
}

/// Run only after the API server has claimed its singleton listener. Opening a
/// second Manager while a server is active must not rewrite in-flight intents.
pub(crate) fn mark_interrupted_operations(db: &Connection) -> Result<()> {
    db.execute(
        "UPDATE consumer_operations SET status='unknown', result=NULL WHERE status='intended'",
        [],
    )?;
    Ok(())
}
