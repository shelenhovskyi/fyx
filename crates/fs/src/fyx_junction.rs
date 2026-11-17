//! fyx: directory junctions on Windows without `cmd /C mklink /J`.
//!
//! Upstream ran `cmd /C mklink /J <link> <target>`. Rust quotes an argument only when it contains a
//! space, tab or quote, so a path with `&`, `|`, `^`, `%`, `<` or `>` reached `cmd` unquoted and could
//! run a command. This sets the mount-point reparse point directly with `DeviceIoControl`, so no shell
//! sees the paths.

use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_MODE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::HSTRING;

/// `FSCTL_SET_REPARSE_POINT` (winioctl.h); defined here to avoid another `windows` feature.
const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
/// `IO_REPARSE_TAG_MOUNT_POINT` (winnt.h): the reparse tag of a directory junction.
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
/// Tag (4 bytes), data length (2) and reserved (2) precede the reparse data.
const REPARSE_HEADER_SIZE: usize = 8;
/// The four `u16` offsets and lengths of `MountPointReparseBuffer`.
const MOUNT_POINT_HEADER_SIZE: usize = 8;

/// Creates a directory junction at `link` that points to the directory `target`. `link` must not
/// exist yet; it is created as an empty directory and turned into the junction.
pub(crate) fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    let buffer = mount_point_reparse_buffer(&std::path::absolute(target)?)?;
    std::fs::create_dir(link)?;
    let result = set_reparse_point(link, &buffer);
    if result.is_err() {
        std::fs::remove_dir(link).ok();
    }
    result
}

fn set_reparse_point(link: &Path, buffer: &[u8]) -> io::Result<()> {
    // SAFETY: plain Win32 calls; `buffer` outlives `DeviceIoControl` and the handle is closed below.
    unsafe {
        let handle = CreateFileW(
            &HSTRING::from(link.as_os_str()),
            GENERIC_WRITE.0,
            FILE_SHARE_MODE::default(),
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(io::Error::other)?;
        let result = DeviceIoControl(
            handle,
            FSCTL_SET_REPARSE_POINT,
            Some(buffer.as_ptr().cast()),
            buffer.len() as u32,
            None,
            0,
            None,
            None,
        )
        .map_err(io::Error::other);
        CloseHandle(handle).ok();
        result
    }
}

/// A `REPARSE_DATA_BUFFER` with a `MountPointReparseBuffer`: the substitute name `\??\C:\target`
/// followed by the print name `C:\target`, each NUL-terminated UTF-16.
fn mount_point_reparse_buffer(target: &Path) -> io::Result<Vec<u8>> {
    let print = print_name(target);
    let mut substitute: Vec<u16> = r"\??\".encode_utf16().collect();
    substitute.extend_from_slice(&print);

    let substitute_bytes = substitute.len() * 2;
    let print_bytes = print.len() * 2;
    // Both names plus their NUL terminators.
    let path_bytes = substitute_bytes + 2 + print_bytes + 2;
    let data_length = MOUNT_POINT_HEADER_SIZE + path_bytes;
    let too_long = || io::Error::new(io::ErrorKind::InvalidInput, "junction target is too long");
    let u16_of = |value: usize| u16::try_from(value).map_err(|_| too_long());

    let mut buffer = Vec::with_capacity(REPARSE_HEADER_SIZE + data_length);
    buffer.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend_from_slice(&u16_of(data_length)?.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes()); // Reserved
    buffer.extend_from_slice(&0u16.to_le_bytes()); // SubstituteNameOffset
    buffer.extend_from_slice(&u16_of(substitute_bytes)?.to_le_bytes());
    buffer.extend_from_slice(&u16_of(substitute_bytes + 2)?.to_le_bytes()); // PrintNameOffset
    buffer.extend_from_slice(&u16_of(print_bytes)?.to_le_bytes());
    let names = substitute
        .iter()
        .chain([&0])
        .chain(print.iter())
        .chain([&0]);
    for unit in names {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(buffer)
}

/// `C:\target` (as UTF-16) for `C:\target` and `\\?\C:\target`; junction targets are local
/// absolute paths.
fn print_name(target: &Path) -> Vec<u16> {
    let wide: Vec<u16> = target.as_os_str().encode_wide().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let verbatim_unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    if wide.starts_with(&verbatim) && !wide.starts_with(&verbatim_unc) {
        wide[verbatim.len()..].to_vec()
    } else {
        wide
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths with characters `cmd` would interpret must become a junction, not a command.
    /// (`|`, `<` and `>` cannot occur in Windows file names.)
    #[test]
    fn junction_with_cmd_special_characters() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("a&calc^x%PATH%!y");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file.txt"), "through the junction").unwrap();
        let link = root.path().join("link&calc");

        create_junction(&link, &target).unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(
            std::fs::read_to_string(link.join("file.txt")).unwrap(),
            "through the junction"
        );
        assert!(!root.path().join("calc").exists());
    }

    #[test]
    fn existing_link_path_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = root.path().join("link");
        std::fs::create_dir(&link).unwrap();

        assert!(create_junction(&link, &target).is_err());
        assert!(link.is_dir(), "an existing directory is left alone");
    }

    #[test]
    fn verbatim_target_prefix_is_dropped() {
        let expected: Vec<u16> = r"C:\x\y".encode_utf16().collect();
        assert_eq!(print_name(Path::new(r"\\?\C:\x\y")), expected);
        assert_eq!(print_name(Path::new(r"C:\x\y")), expected);
    }
}
