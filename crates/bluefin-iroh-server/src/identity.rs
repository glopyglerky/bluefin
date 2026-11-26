//! Private, exclusively leased persistent endpoint identity. Never formatted or logged.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

pub(crate) fn open(path: &Path) -> Result<(iroh::SecretKey, File)> {
    ensure!(path.is_absolute(), "identity path must be absolute");
    let mut dir = File::open("/")?;
    let components = path.components().collect::<Vec<_>>();
    let Some(Component::Normal(name)) = components.last() else {
        anyhow::bail!("identity filename required")
    };
    for component in &components[1..components.len() - 1] {
        let Component::Normal(part) = component else {
            anyhow::bail!("identity path must be normalized")
        };
        let part = CString::new(part.as_encoded_bytes())?;
        // SAFETY: live directory descriptor and NUL-terminated component; owned fd on success.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "identity directory unavailable");
        // SAFETY: openat returned a unique owned fd.
        dir = unsafe { File::from_raw_fd(fd) };
    }
    let meta = dir.metadata()?;
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    ensure!(
        meta.uid() == uid && meta.mode() & 0o777 == 0o700,
        "identity directory must be owned and mode 0700"
    );
    let name = CString::new(name.as_encoded_bytes())?;
    let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    // SAFETY: live directory fd, valid filename, restrictive creation mode.
    let mut fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    };
    let created = fd >= 0;
    if !created {
        ensure!(
            std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists,
            "identity create failed"
        );
        // SAFETY: same directory and component, no-follow remains enforced.
        fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    }
    ensure!(fd >= 0, "identity open failed");
    // SAFETY: openat returned a unique owned fd.
    let mut file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == uid && meta.mode() & 0o777 == 0o600 && meta.nlink() == 1,
        "identity must be an owned private regular file"
    );
    // SAFETY: flock operates on this owned, live descriptor. Lease retained with endpoint.
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "identity already leased"
    );
    let key = if created {
        let key = iroh::SecretKey::generate();
        file.write_all(&key.to_bytes())
            .context("identity write failed")?;
        file.sync_all()?;
        dir.sync_all()?;
        key
    } else {
        ensure!(meta.len() == 32, "invalid identity length");
        let mut bytes = [0; 32];
        file.read_exact(&mut bytes)?;
        let key = iroh::SecretKey::from_bytes(&bytes);
        bytes.fill(0);
        key
    };
    Ok((key, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn identity_is_stable_private_and_exclusive() {
        let d = tempfile::tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let p = d.path().join("key");
        let (a, lease) = open(&p).unwrap();
        assert!(open(&p).is_err());
        drop(lease);
        let (b, lease) = open(&p).unwrap();
        assert_eq!(a.public(), b.public());
        drop(lease);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open(&p).is_err());
        std::fs::remove_file(&p).unwrap();
        symlink("absent", &p).unwrap();
        assert!(open(&p).is_err());
        std::fs::remove_file(&p).unwrap();
        std::fs::write(&p, b"short").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(open(&p).is_err());
        let alias = d.path().join("alias");
        symlink(d.path(), &alias).unwrap();
        assert!(open(&alias.join("key")).is_err());
        std::fs::remove_file(&p).unwrap();
        let (_, lease) = open(&p).unwrap();
        drop(lease);
        let hardlink = d.path().join("second-name");
        std::fs::hard_link(&p, &hardlink).unwrap();
        assert!(open(&p).is_err());
        std::fs::remove_file(hardlink).unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(open(&p).is_err());
    }
}
