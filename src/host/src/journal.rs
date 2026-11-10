use crate::{JournalId, Manager, Result, VmId};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    pub sequence: u64,
    pub id: JournalId,
    pub vm: VmId,
    pub generation: u64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub previous_sha256: String,
    pub sha256: String,
}

fn digest(entry: &JournalEntry) -> String {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "sequence": entry.sequence,
        "id": entry.id,
        "vm": entry.vm,
        "generation": entry.generation,
        "kind": entry.kind,
        "payload": entry.payload,
        "previous_sha256": entry.previous_sha256,
    }))
    .expect("journal serialization is infallible");
    Sha256::digest(bytes)
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sqlite_integer(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

impl Manager {
    pub fn append_journal(
        &self,
        vm: VmId,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<JournalEntry> {
        if kind.is_empty()
            || kind.len() > 64
            || !kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            || serde_json::to_vec(&payload)?.len() > 16384
        {
            return Err("invalid journal entry".into());
        }
        let generation = self.get(vm)?.generation;
        let tx = self.db.unchecked_transaction()?;
        let previous = tx
            .query_row(
                "SELECT sequence,sha256 FROM journal ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let previous = previous
            .map(|(sequence, hash)| -> rusqlite::Result<(u64, String)> {
                Ok((sqlite_integer(sequence)?, hash))
            })
            .transpose()?;
        let sequence = previous.as_ref().map_or(0, |(sequence, _)| *sequence) + 1;
        let mut entry = JournalEntry {
            sequence,
            id: JournalId::from(Uuid::new_v4()),
            vm,
            generation,
            kind: kind.into(),
            payload,
            previous_sha256: previous.map_or_else(|| "0".repeat(64), |(_, hash)| hash),
            sha256: String::new(),
        };
        entry.sha256 = digest(&entry);
        tx.execute(
            "INSERT INTO journal VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                i64::try_from(entry.sequence)
                    .map_err(|_| crate::Error::from("journal sequence exceeds SQLite"))?,
                entry.id.to_string(),
                entry.vm.to_string(),
                i64::try_from(entry.generation)
                    .map_err(|_| crate::Error::from("generation exceeds SQLite"))?,
                entry.kind,
                entry.payload.to_string(),
                entry.previous_sha256,
                entry.sha256
            ],
        )?;
        tx.commit()?;
        Ok(entry)
    }

    pub fn journal(&self) -> Result<Vec<JournalEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT sequence,id,vm,generation,kind,payload,previous_sha256,sha256 FROM journal ORDER BY sequence",
        )?;
        let rows = stmt.query_map([], |row| {
            let payload: String = row.get(5)?;
            Ok(JournalEntry {
                sequence: sqlite_integer(row.get(0)?)?,
                id: JournalId::from(
                    Uuid::parse_str(&row.get::<_, String>(1)?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                ),
                vm: VmId::from(
                    Uuid::parse_str(&row.get::<_, String>(2)?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                ),
                generation: sqlite_integer(row.get(3)?)?,
                kind: row.get(4)?,
                payload: serde_json::from_str(&payload)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                previous_sha256: row.get(6)?,
                sha256: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn verify_journal(&self) -> Result<bool> {
        let entries = self.journal()?;
        let mut previous = "0".repeat(64);
        for (index, entry) in entries.iter().enumerate() {
            if entry.sequence != index as u64 + 1
                || entry.previous_sha256 != previous
                || entry.sha256 != digest(entry)
            {
                return Ok(false);
            }
            previous = entry.sha256.clone();
        }
        Ok(true)
    }
}
