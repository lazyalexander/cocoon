use super::*;
use std::{collections::HashSet, path::Path};

fn size(path: &Path, seen: &mut HashSet<(u64, u64)>, remaining: &mut usize) -> Result<(u64, u64)> {
    if *remaining == 0 {
        return Err("resource inventory limit".into());
    }
    *remaining -= 1;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_symlink() {
        return Err("symlink resource retained".into());
    }
    if metadata.is_dir() {
        let mut total = (0, 0);
        for entry in fs::read_dir(path)? {
            let next = size(&entry?.path(), seen, remaining)?;
            total.0 += next.0;
            total.1 += next.1;
        }
        Ok(total)
    } else if metadata.is_file() {
        Ok((
            metadata.len(),
            if seen.insert((metadata.dev(), metadata.ino())) {
                metadata.blocks() * 512
            } else {
                0
            },
        ))
    } else {
        Err("special resource retained".into())
    }
}
impl Manager {
    pub(crate) fn unregistered_resource_count(&self) -> Result<usize> {
        let images: HashSet<String> = {
            let db = self.db();
            let mut q = db.prepare("SELECT id FROM images")?;
            q.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let vms: HashSet<String> = self.list()?.iter().map(|v| v.id.to_string()).collect();
        let mut count = 0;
        for (name, known) in [("images", images), ("vms", vms)] {
            for entry in fs::read_dir(self.root.join(name))?.take(8193) {
                let entry = entry?;
                if !known.contains(&entry.file_name().to_string_lossy().into_owned()) {
                    count += 1;
                }
                if count > 8192 {
                    return Err("unregistered inventory limit".into());
                }
            }
        }
        Ok(count)
    }
    /// Local operator inventory only. Paths, tokens and raw execution records are excluded.
    pub fn resource_inventory(&self) -> Result<Value> {
        let mut resources = Vec::new();
        let ids: Vec<String> = {
            let db = self.db();
            let mut q = db.prepare("SELECT id FROM images ORDER BY id")?;
            q.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        if ids.len() > 4096 {
            return Err("image inventory limit".into());
        }
        for id in ids {
            let image = ImageId::from(
                uuid::Uuid::parse_str(&id)
                    .map_err(|_| crate::Error::from("invalid registered image"))?,
            );
            let manifest = self.image(image)?;
            let retired = manifest["retired"] == true;
            let mut reasons = if retired {
                vec!["retired".to_string()]
            } else {
                self.retirement_blockers(image, None)?
            };
            let bytes = if retired {
                Some((0, 0))
            } else {
                size(
                    &self.root.join("images").join(&id),
                    &mut HashSet::new(),
                    &mut 100_000,
                )
                .ok()
            };
            if bytes.is_none() {
                reasons.push("unverifiable_storage".into());
            }
            let references: Vec<_> = self
                .list()?
                .iter()
                .filter(|v| v.image == image)
                .map(|v| v.id.to_string())
                .collect();
            resources.push(json!({"kind":"image","id":id,"sha256":self.image_content_digest(image)?,"approved":manifest["approved"],"retired":retired,
                "references":references,"logical_bytes":bytes.map(|v|v.0),"allocated_bytes":bytes.map(|v|v.1),"reclaimable":reasons.is_empty(),"blocked_by":reasons}));
        }
        for vm in self.list()? {
            let mut reasons = Vec::new();
            if self.pending_maintenance_blockers(None)? {
                reasons.push("unresolved_operations".to_string());
            }
            if self.validate_reclaim(vm.id).is_err() {
                reasons.push("termination_or_custody_unconfirmed".to_string());
            }
            let bytes = size(
                &self.root.join("vms").join(vm.id.to_string()),
                &mut HashSet::new(),
                &mut 100_000,
            )
            .ok();
            if bytes.is_none() {
                reasons.push("unverifiable_storage".into());
            }
            resources.push(json!({"kind":"vm","id":vm.id,"image":vm.image,"generation":vm.generation,"revision":vm.revision,"state":vm.state,
                "logical_bytes":bytes.map(|v|v.0),"allocated_bytes":bytes.map(|v|v.1),"reclaimable":reasons.is_empty(),"blocked_by":reasons}));
        }
        Ok(
            json!({"schema":"cocoon-resource-inventory-v1","capabilities":self.maintenance_capabilities(),"resources":resources,
            "allocated_bytes_are_estimates":true,"automatic_gc":false,"unknown_resources_retained":true,"unregistered_resource_count":self.unregistered_resource_count()?}),
        )
    }
}
