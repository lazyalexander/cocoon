//! Trusted local host administration. No guest-facing lifecycle authority.
//!
//! Identities cannot be accidentally substituted across resource kinds:
//! ```compile_fail
//! use cocoon_host::{ImageId, VmId};
//! let image = ImageId::from(uuid::Uuid::new_v4());
//! let vm: VmId = image;
//! ```
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

mod api;
mod disk;
mod error;
mod journal;
mod lifecycle;
pub mod persistence;
mod project;
mod types;
pub use api::{HostOperation, HostRequest, HostResponse};
use disk::{copy_new, hash, sync_directory};
pub use error::{Error, Result};
pub use journal::JournalEntry;
pub use lifecycle::ExecutionRecord;
pub use types::*;
/// Only trusted host code may construct this interface. Socket ACLs are not
/// isolation from arbitrary code running under the same macOS user.
pub struct Manager {
    root: PathBuf,
    db: Connection,
    children: Arc<Mutex<HashMap<AttemptId, Child>>>,
}
impl Manager {
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err("state directory must be absolute".into());
        }
        if !root.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(root)?;
        }
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
            return Err("state directory must be a private directory (mode 0700)".into());
        }
        for name in ["images", "vms"] {
            let dir = root.join(name);
            if !dir.exists() {
                fs::DirBuilder::new().mode(0o700).create(&dir)?;
            }
            let m = fs::symlink_metadata(&dir)?;
            if !m.is_dir() || m.permissions().mode() & 0o077 != 0 {
                return Err("unsafe state subdirectory".into());
            }
        }
        let db = Connection::open(root.join("state.sqlite3"))?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 2 {
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
            PRAGMA user_version=2; COMMIT;")?;
        Ok(Self {
            root: root.to_owned(),
            db,
            children: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    fn begin(&self, id: OperationId, kind: &str, target: impl std::fmt::Display) -> Result<()> {
        self.db.execute(
            "INSERT INTO operations VALUES (?1,?2,?3,'pending')",
            params![id.to_string(), kind, target.to_string()],
        )?;
        Ok(())
    }
    pub fn operations(&self) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self
            .db
            .prepare("SELECT id,kind,target,status FROM operations ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"target":r.get::<_,String>(2)?,"status":r.get::<_,String>(3)?})))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn import_image(
        &self,
        kernel: &Path,
        initrd: &Path,
        disk: &Path,
        provenance: &str,
    ) -> Result<ImageId> {
        if provenance.trim().is_empty() {
            return Err("provenance and license information required".into());
        }
        let id = ImageId::from(Uuid::new_v4());
        let operation = OperationId::from(Uuid::new_v4());
        self.begin(operation, "import_image", id)?;
        let dir = self.root.join("images").join(id.to_string());
        fs::create_dir(&dir)?;
        let mut files = serde_json::Map::new();
        for (name, source) in [("kernel", kernel), ("initrd", initrd), ("root.raw", disk)] {
            let target = dir.join(name);
            let copy = copy_new(source, &target)?;
            files.insert(
                name.into(),
                serde_json::json!({"sha256": hash(&target)?, "copy":copy}),
            );
            fs::set_permissions(&target, fs::Permissions::from_mode(0o400))?;
        }
        let manifest = serde_json::json!({"id":id,"files":files,"provenance":provenance,"trusted":false,"boot_qualified":false});
        sync_directory(&dir)?;
        sync_directory(&self.root.join("images"))?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO images VALUES (?1,?2)",
            params![id.to_string(), manifest.to_string()],
        )?;
        tx.execute(
            "UPDATE operations SET status='complete' WHERE id=?1",
            [operation.to_string()],
        )?;
        tx.commit()?;
        Ok(id)
    }
    pub fn image(&self, id: ImageId) -> Result<serde_json::Value> {
        let record: String = self.db.query_row(
            "SELECT manifest FROM images WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&record)?)
    }
    fn verify_image(&self, id: ImageId) -> Result<()> {
        let manifest = self.image(id)?;
        for name in ["kernel", "initrd", "root.raw"] {
            let actual = hash(&self.root.join("images").join(id.to_string()).join(name))?;
            if manifest["files"][name]["sha256"].as_str() != Some(&actual) {
                return Err("image integrity mismatch".into());
            }
        }
        Ok(())
    }
    pub fn create(
        &self,
        id: VmId,
        image: ImageId,
        owner: &str,
        policy: FinishPolicy,
    ) -> Result<Vm> {
        if owner.is_empty() || owner.len() > 256 {
            return Err("invalid owner".into());
        }
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT record FROM vms WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(record) = existing {
            let vm: Vm = serde_json::from_str(&record)?;
            if vm.image == image && vm.owner == owner && vm.policy == policy {
                return Ok(vm);
            }
            return Err("VM identity already exists with different parameters".into());
        }
        self.verify_image(image)?;
        // A pending create is never automatically replayed after an unknown result.
        self.begin(OperationId::from(id.raw()), "create_vm", id)?;
        let dir = self.root.join("vms").join(id.to_string());
        fs::create_dir(&dir)?;
        let disk_copy = copy_new(
            &self
                .root
                .join("images")
                .join(image.to_string())
                .join("root.raw"),
            &dir.join("root.raw"),
        )?;
        let vm = Vm {
            id,
            owner: owner.into(),
            image,
            policy,
            state: VmState::Prepared,
            network: false,
            vcpus: 1,
            memory_mib: 512,
            generation: 0,
            active_attempt: None,
            active_lease: None,
            disk_copy: Some(disk_copy),
        };
        sync_directory(&dir)?;
        sync_directory(&self.root.join("vms"))?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO vms VALUES (?1,?2)",
            params![vm.id.to_string(), serde_json::to_string(&vm)?],
        )?;
        tx.execute(
            "UPDATE operations SET status='complete' WHERE id=?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(vm)
    }
    pub fn get(&self, id: VmId) -> Result<Vm> {
        let record: String = self.db.query_row(
            "SELECT record FROM vms WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&record)?)
    }
    pub fn list(&self) -> Result<Vec<Vm>> {
        let mut stmt = self.db.prepare("SELECT record FROM vms ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn start(&self, id: VmId) -> Result<()> {
        let runner =
            std::env::var_os("COCOON_RUNNER").ok_or("COCOON_RUNNER must name the signed runner")?;
        self.start_with_runner(id, Path::new(&runner)).map(|_| ())
    }
}
