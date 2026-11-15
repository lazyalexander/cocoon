use super::*;

fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Manager {
    pub(super) fn validate_image_action(
        &self,
        request: RequestId,
        action: &ImageAction,
    ) -> Result<()> {
        let check = || self.check_image_cancel(request);
        check()?;
        match action {
            ImageAction::ImportCandidate {
                environment,
                kernel,
                initrd,
                disk,
                kernel_sha256,
                initrd_sha256,
                disk_sha256,
            } => {
                if !hex(environment) {
                    return Err("invalid environment identity".into());
                }
                for (path, sha) in [
                    (kernel, kernel_sha256),
                    (initrd, initrd_sha256),
                    (disk, disk_sha256),
                ] {
                    if !path.is_absolute()
                        || !fs::symlink_metadata(path)?.is_file()
                        || !hex(sha)
                        || crate::disk::hash_controlled(path, &check)? != *sha
                    {
                        return Err("image source digest mismatch".into());
                    }
                }
            }
            ImageAction::SaveCandidate {
                vm,
                generation,
                disk_sha256,
            }
            | ImageAction::Rollback {
                vm,
                generation,
                disk_sha256,
                ..
            } => {
                let current = self.get(*vm)?;
                if current.generation != *generation || !hex(disk_sha256) {
                    return Err("image source generation mismatch".into());
                }
                self.image_source_ready(*vm)?;
                if crate::disk::hash_controlled(
                    &self.root.join("vms").join(vm.to_string()).join("root.raw"),
                    &check,
                )? != *disk_sha256
                {
                    return Err("working disk changed".into());
                }
            }
            _ => {}
        }
        match action {
            ImageAction::PromoteCandidate { image, sha256 } => {
                self.match_image(*image, sha256, &check)?;
                let manifest = self.image(*image)?;
                if manifest["approved"] != false
                    || (manifest["source_vm"].is_null() && manifest["source_environment"].is_null())
                {
                    return Err("image is not a candidate".into());
                }
            }
            ImageAction::Branch {
                image,
                sha256,
                vm,
                owner,
            } => {
                self.match_image(*image, sha256, &check)?;
                if self.image(*image)?["approved"] != true
                    || owner.is_empty()
                    || owner.len() > 256
                    || self.get(*vm).is_ok()
                    || self.root.join("vms").join(vm.to_string()).exists()
                {
                    return Err("branch target or source unavailable".into());
                }
            }
            ImageAction::Rollback { image, sha256, .. } => {
                self.match_image(*image, sha256, &check)?;
                if self.image(*image)?["approved"] != true {
                    return Err("rollback image is not approved".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn match_image(
        &self,
        id: ImageId,
        expected: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        if !hex(expected) || self.image_content_digest(id)? != expected {
            return Err("image identity changed".into());
        }
        self.verify_image_controlled(id, check)
    }
}
