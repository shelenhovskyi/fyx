//! Writing a discovery file where another user cannot read it or redirect it (design wit-0.4
//! §B.8): each folder under the trusted base is opened without following links and must belong
//! to this user; folders Fyx creates are `0700` (an owner-only ACL on Windows), the file is `0600`,
//! written under a temporary name in its folder and renamed.
//!
//! The base (an account dir, the home folder, the temp folder) is trusted as it stands. Under it,
//! a folder another user owns is refused even when it is a plain folder: `/tmp` is shared, and a
//! pre-created `/tmp/gemini/ide` would let its owner read the token. Once a folder is this user's,
//! no other user can swap what is inside it, so checking each one in turn is enough.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::fill::Target;

fn refused(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{}: {why}", path.display()),
    )
}

/// The folder for `target`, its missing folders created.
pub(crate) fn prepare_dir(target: &Target) -> io::Result<PathBuf> {
    let mut dir = trusted_base(&target.base)?;
    for folder in &target.folders {
        dir.push(folder);
        match platform::open_dir(&dir) {
            Ok(handle) => platform::check_owned_dir(&handle, &dir)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                platform::create_private_dir(&dir)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(dir)
}

fn trusted_base(base: &Path) -> io::Result<PathBuf> {
    // On macOS `/tmp` is a link to `/private/tmp`; the base may be one, the folders under it not.
    #[cfg(unix)]
    let base = fs::canonicalize(base)?;
    #[cfg(not(unix))]
    let base = base.to_path_buf();
    if !fs::metadata(&base)?.is_dir() {
        return Err(refused(&base, "not a folder"));
    }
    Ok(base)
}

/// Writes `contents` to `dir/name`, readable by this user only, replacing what was there.
pub(crate) fn write_file(dir: &Path, name: &str, contents: &[u8]) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = dir.join(format!(
        ".fyx-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let path = dir.join(name);
    let written = (|| {
        let mut file = platform::create_private_file(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)
    })();
    if written.is_err() {
        fs::remove_file(&temporary).ok();
    }
    written.map(|()| path)
}

/// Removes a file Fyx wrote, unless it is gone or something else (a link, a folder) is there now.
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !platform::is_reparse(&metadata) => {
            fs::remove_file(path)
        }
        Ok(_) => Err(refused(path, "not a plain file any more; left alone")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
mod platform {
    use std::fs::{DirBuilder, File, Metadata, OpenOptions};
    use std::io;
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
    use std::path::Path;

    use super::refused;

    pub(super) fn open_dir(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| match error.raw_os_error() {
                Some(libc::ELOOP) => refused(path, "a link, not a folder"),
                Some(libc::ENOTDIR) => refused(path, "not a folder"),
                _ => error,
            })
    }

    pub(super) fn check_owned_dir(handle: &File, path: &Path) -> io::Result<()> {
        let metadata = handle.metadata()?;
        if !metadata.is_dir() {
            return Err(refused(path, "not a folder"));
        }
        // SAFETY: geteuid has no preconditions.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(refused(path, "belongs to another user"));
        }
        Ok(())
    }

    pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
        DirBuilder::new().mode(0o700).create(path)?;
        check_owned_dir(&open_dir(path)?, path)
    }

    pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
    }

    pub(super) fn is_reparse(_metadata: &Metadata) -> bool {
        false
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::os::windows::io::AsRawHandle as _;
    use std::path::Path;

    use windows::Win32::Foundation::{
        CloseHandle, ERROR_SUCCESS, GENERIC_WRITE, HANDLE, HLOCAL, LocalFree, WIN32_ERROR,
    };
    use windows::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, GetSecurityInfo, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS,
        SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows::Win32::Security::{
        ACE_FLAGS, ACL, DACL_SECURITY_INFORMATION, EqualSid, GetTokenInformation, NO_INHERITANCE,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_INFORMATION_CLASS, TOKEN_OWNER,
        TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, READ_CONTROL, WRITE_DAC,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::PWSTR;

    use super::refused;

    fn handle(file: &File) -> HANDLE {
        HANDLE(file.as_raw_handle())
    }

    fn check(error: WIN32_ERROR) -> io::Result<()> {
        if error == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(error.0 as i32))
        }
    }

    pub(super) fn is_reparse(metadata: &Metadata) -> bool {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
    }

    pub(super) fn open_dir(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .access_mode(READ_CONTROL.0 | WRITE_DAC.0 | FILE_READ_ATTRIBUTES.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
    }

    pub(super) fn check_owned_dir(file: &File, path: &Path) -> io::Result<()> {
        let metadata = file.metadata()?;
        if is_reparse(&metadata) {
            return Err(refused(path, "a link or junction, not a folder"));
        }
        if !metadata.is_dir() {
            return Err(refused(path, "not a folder"));
        }
        if !owned_by_this_user(file)? {
            return Err(refused(path, "belongs to another user"));
        }
        Ok(())
    }

    pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
        std::fs::create_dir(path)?;
        let file = open_dir(path)?;
        check_owned_dir(&file, path)?;
        owner_only(&file, SUB_CONTAINERS_AND_OBJECTS_INHERIT)
    }

    pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .access_mode(GENERIC_WRITE.0 | READ_CONTROL.0 | WRITE_DAC.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)?;
        owner_only(&file, NO_INHERITANCE)?;
        Ok(file)
    }

    /// The token's user and its default owner (an elevated admin's files belong to
    /// Administrators): the SIDs a file this process creates gets as its owner.
    struct Token {
        user: Vec<u8>,
        owner: Vec<u8>,
    }

    impl Token {
        fn current() -> io::Result<Self> {
            let mut token = HANDLE::default();
            // SAFETY: the pseudo-handle of this process needs no closing; `token` is closed below.
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;
            let info = |class: TOKEN_INFORMATION_CLASS| -> io::Result<Vec<u8>> {
                let mut size = 0;
                // SAFETY: a size query with no buffer.
                unsafe { GetTokenInformation(token, class, None, 0, &mut size) }.ok();
                let mut buffer = vec![0u8; size as usize];
                // SAFETY: the buffer is `size` bytes.
                unsafe {
                    GetTokenInformation(
                        token,
                        class,
                        Some(buffer.as_mut_ptr().cast()),
                        size,
                        &mut size,
                    )
                }?;
                Ok(buffer)
            };
            let result = (|| {
                Ok(Self {
                    user: info(TokenUser)?,
                    owner: info(TokenOwner)?,
                })
            })();
            // SAFETY: opened above.
            unsafe { CloseHandle(token) }.ok();
            result
        }

        fn user_sid(&self) -> PSID {
            // SAFETY: the buffer holds a TOKEN_USER that GetTokenInformation wrote.
            unsafe { (*self.user.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        }

        fn owner_sid(&self) -> PSID {
            // SAFETY: the buffer holds a TOKEN_OWNER that GetTokenInformation wrote.
            unsafe { (*self.owner.as_ptr().cast::<TOKEN_OWNER>()).Owner }
        }
    }

    /// Frees a security descriptor or an ACL that Windows allocated.
    struct Local(*mut c_void);

    impl Drop for Local {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: allocated by GetSecurityInfo or SetEntriesInAclW.
                unsafe { LocalFree(Some(HLOCAL(self.0))) };
            }
        }
    }

    fn owned_by_this_user(file: &File) -> io::Result<bool> {
        let token = Token::current()?;
        let mut owner = PSID::default();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the out-pointers are valid; the descriptor holding `owner` is freed by `Local`.
        check(unsafe {
            GetSecurityInfo(
                handle(file),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                None,
                None,
                Some(&mut descriptor),
            )
        })?;
        let _descriptor = Local(descriptor.0);
        // SAFETY: both SIDs are valid while `token` and `_descriptor` live.
        let same = |sid: PSID| unsafe { EqualSid(owner, sid) }.is_ok();
        Ok(same(token.user_sid()) || same(token.owner_sid()))
    }

    /// Replaces the ACL with one entry: full access for this user, not inherited from above.
    fn owner_only(file: &File, inheritance: ACE_FLAGS) -> io::Result<()> {
        let token = Token::current()?;
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS.0,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: PWSTR(token.user_sid().0.cast()),
            },
        };
        let mut acl: *mut ACL = std::ptr::null_mut();
        // SAFETY: one valid entry; the new ACL is freed by `Local`.
        check(unsafe { SetEntriesInAclW(Some(&[access]), None, &mut acl) })?;
        let _acl = Local(acl.cast());
        // SAFETY: the handle was opened with WRITE_DAC; the ACL is valid.
        check(unsafe {
            SetSecurityInfo(
                handle(file),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            )
        })
    }

    #[cfg(test)]
    pub(super) fn ace_count(path: &Path) -> io::Result<u32> {
        use windows::Win32::Security::{
            ACL_SIZE_INFORMATION, AclSizeInformation, GetAclInformation,
        };
        let file = OpenOptions::new()
            .access_mode(READ_CONTROL.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)?;
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: valid out-pointers; the descriptor is freed by `Local`.
        check(unsafe {
            GetSecurityInfo(
                handle(&file),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                Some(&mut descriptor),
            )
        })?;
        let _descriptor = Local(descriptor.0);
        let mut size = ACL_SIZE_INFORMATION::default();
        // SAFETY: `dacl` points into the descriptor, which lives until the end.
        unsafe {
            GetAclInformation(
                dacl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        }?;
        Ok(size.AceCount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(base: &Path, folders: &[&str]) -> Target {
        Target {
            base: base.to_path_buf(),
            folders: folders.iter().map(|folder| (*folder).to_owned()).collect(),
        }
    }

    #[test]
    fn folders_are_created_private_and_the_file_replaced_whole() {
        let base = tempfile::tempdir().unwrap();
        let dir = prepare_dir(&target(base.path(), &["gemini", "ide"])).unwrap();
        assert!(dir.ends_with(Path::new("gemini").join("ide")));
        // A second time finds the same folders, now this user's.
        assert_eq!(
            prepare_dir(&target(base.path(), &["gemini", "ide"])).unwrap(),
            dir
        );

        let path = write_file(&dir, "1.lock", b"one").unwrap();
        let path_again = write_file(&dir, "1.lock", b"two").unwrap();
        assert_eq!(path, path_again);
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["1.lock"], "no temporary file is left");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.parent().unwrap()), 0o700);
            assert_eq!(mode(&path), 0o600);
        }
        #[cfg(windows)]
        {
            assert_eq!(platform::ace_count(&dir).unwrap(), 1);
            assert_eq!(platform::ace_count(&path).unwrap(), 1);
        }

        remove_file(&path).unwrap();
        assert!(!path.exists());
        remove_file(&path).unwrap();
    }

    #[test]
    fn a_link_in_the_path_is_refused_and_a_file_is_not_a_folder() {
        let base = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(base.path().join("plain"), b"").unwrap();
        assert!(prepare_dir(&target(base.path(), &["plain"])).is_err());

        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(elsewhere.path(), base.path().join("ide")).is_ok();
        // A plain test, no executor: blocking on `mklink` is fine.
        #[cfg(windows)]
        #[allow(clippy::disallowed_methods)]
        let linked = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(base.path().join("ide"))
            .arg(elsewhere.path())
            .output()
            .is_ok_and(|output| output.status.success());
        assert!(linked, "could not make a link to test with");
        let error = prepare_dir(&target(base.path(), &["ide", "x"])).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(!elsewhere.path().join("x").exists());

        // A link put where a written file was is left alone.
        #[cfg(unix)]
        {
            let link = base.path().join("1.lock");
            std::os::unix::fs::symlink(elsewhere.path(), &link).unwrap();
            assert!(remove_file(&link).is_err());
            assert!(elsewhere.path().exists());
        }
    }
}
