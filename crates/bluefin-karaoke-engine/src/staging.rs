//! Directory-descriptor-relative private staging. Linux publication is an atomic
//! NOREPLACE rename of the complete directory. No caller-derived filenames.
use crate::{Cancellation, Error};
use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
use std::{fs::File, io::Write, os::unix::fs::MetadataExt};

#[derive(Clone, Copy)]
pub enum Stem {
    LeadVocal,
    Accompaniment,
}
impl Stem {
    fn filename(self) -> &'static str {
        match self {
            Self::LeadVocal => "lead-vocal.wav",
            Self::Accompaniment => "accompaniment.wav",
        }
    }
}
const FILES: [&str; 3] = ["lead-vocal.wav", "accompaniment.wav", "provenance.json"];

/// Opened private directory capability, selected by trusted server configuration.
/// Parent must open every path component without following symlinks. This API
/// accepts only the resulting directory descriptor, never an untrusted path.
pub struct StagingRoot {
    dir: File,
}
impl StagingRoot {
    pub fn from_opened_directory(dir: File) -> Result<Self, Error> {
        let m = dir.metadata()?;
        if !m.is_dir()
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.mode() & 0o777 != 0o700
        {
            return Err(Error::InvalidStaging);
        }
        Ok(Self { dir })
    }
    pub(crate) fn begin(&self) -> Result<PublishedTask, Error> {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|e| std::io::Error::other(e.to_string()))?;
        let suffix: String = nonce.iter().map(|v| format!("{v:02x}")).collect();
        let name = format!(".pending-{suffix}");
        let root = self.dir.try_clone()?;
        fs::mkdirat(&root, &name, Mode::from_raw_mode(0o700))?;
        match fs::openat(
            &root,
            &name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(dir) => Ok(PublishedTask {
                root,
                dir: File::from(dir),
                name,
                removed: false,
            }),
            Err(e) => {
                fs::unlinkat(&root, &name, AtFlags::REMOVEDIR)?;
                Err(e.into())
            }
        }
    }
}

/// Owns the private generated directory until consumed. Dropping it removes the
/// three known output files and directory. Call `remove` to observe cleanup errors.
/// Output descriptors stay pinned if the parent renames a higher directory.
pub struct PublishedTask {
    root: File,
    dir: File,
    name: String,
    removed: bool,
}
impl PublishedTask {
    pub fn directory_name(&self) -> &str {
        &self.name
    }
    pub fn open_stem(&self, stem: Stem) -> Result<File, Error> {
        self.open(stem.filename())
    }
    pub fn open_provenance(&self) -> Result<File, Error> {
        self.open("provenance.json")
    }
    fn open(&self, name: &str) -> Result<File, Error> {
        let file = File::from(fs::openat(
            &self.dir,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?);
        let m = file.metadata()?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.mode() & 0o777 != 0o600
        {
            return Err(Error::InvalidStaging);
        }
        Ok(file)
    }
    pub(crate) fn write(
        &self,
        name: &str,
        bytes: &[u8],
        cancel: &Cancellation,
    ) -> Result<(), Error> {
        if !FILES.contains(&name) {
            return Err(Error::InvalidStaging);
        }
        let mut file = File::from(fs::openat(
            &self.dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_raw_mode(0o600),
        )?);
        for chunk in bytes.chunks(65536) {
            cancel.check()?;
            file.write_all(chunk)?;
        }
        file.sync_all()?;
        Ok(())
    }
    pub(crate) fn publish(&mut self, cancel: &Cancellation) -> Result<(), Error> {
        cancel.check()?;
        for name in FILES {
            self.open(name)?;
        }
        self.dir.sync_all()?;
        let final_name = self.name.replacen(".pending-", "processed-", 1);
        fs::renameat_with(
            &self.root,
            &self.name,
            &self.root,
            &final_name,
            RenameFlags::NOREPLACE,
        )?;
        self.name = final_name;
        self.root.sync_all()?;
        cancel.check()?;
        Ok(())
    }
    fn cleanup(&mut self) -> Result<(), std::io::Error> {
        if self.removed {
            return Ok(());
        }
        for name in FILES {
            match fs::unlinkat(&self.dir, name, AtFlags::empty()) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => {}
                Err(e) => return Err(e.into()),
            }
        }
        fs::unlinkat(&self.root, &self.name, AtFlags::REMOVEDIR)?;
        self.removed = true;
        self.root.sync_all()?;
        Ok(())
    }
    pub fn remove(mut self) -> Result<(), Error> {
        self.cleanup()?;
        Ok(())
    }
    pub(crate) fn abort(mut self, cause: Error) -> Error {
        match self.cleanup() {
            Ok(()) => cause,
            Err(e) => Error::Cleanup {
                cause: Box::new(cause),
                cleanup: e,
            },
        }
    }
}
impl Drop for PublishedTask {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn root() -> (tempfile::TempDir, StagingRoot) {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let cap = StagingRoot::from_opened_directory(File::open(root.path()).unwrap()).unwrap();
        (root, cap)
    }
    #[test]
    fn atomic_private_publish_cleanup_and_no_follow() {
        let (root, cap) = root();
        let cancel = Cancellation::default();
        let mut task = cap.begin().unwrap();
        assert!(task.directory_name().starts_with(".pending-"));
        for name in FILES {
            task.write(name, b"fixture", &cancel).unwrap();
        }
        task.publish(&cancel).unwrap();
        assert!(task.directory_name().starts_with("processed-"));
        assert!(task.write("lead-vocal.wav", b"overwrite", &cancel).is_err());
        task.remove().unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        let task = cap.begin().unwrap();
        let external = root.path().join("external");
        std::fs::write(&external, b"preserve").unwrap();
        symlink(
            &external,
            root.path()
                .join(task.directory_name())
                .join("lead-vocal.wav"),
        )
        .unwrap();
        assert!(task.write("lead-vocal.wav", b"bad", &cancel).is_err());
        assert!(task.open_stem(Stem::LeadVocal).is_err());
        task.remove().unwrap();
        assert_eq!(std::fs::read(external).unwrap(), b"preserve");
    }
    #[test]
    fn rejects_shared_root_and_cleans_cancelled_partial() {
        let (root, cap) = root();
        let task = cap.begin().unwrap();
        let c = Cancellation::default();
        c.cancel();
        assert!(task.write("lead-vocal.wav", &[0; 65536], &c).is_err());
        drop(task);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(StagingRoot::from_opened_directory(File::open(root.path()).unwrap()).is_err());
    }
    #[test]
    fn never_replaces_existing_publication() {
        let (root, cap) = root();
        let c = Cancellation::default();
        let mut task = cap.begin().unwrap();
        for name in FILES {
            task.write(name, b"fixture", &c).unwrap();
        }
        let dest = task.directory_name().replacen(".pending-", "processed-", 1);
        std::fs::create_dir(root.path().join(&dest)).unwrap();
        std::fs::write(root.path().join(&dest).join("unrelated"), b"keep").unwrap();
        assert!(task.publish(&c).is_err());
        drop(task);
        assert_eq!(
            std::fs::read(root.path().join(dest).join("unrelated")).unwrap(),
            b"keep"
        );
    }
}
