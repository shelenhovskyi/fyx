//! What a `.fyxext` package may contain (owner's decision Q1, 2026-11-19).
//!
//! A package with a Zed `extension.wasm` would also run as a Zed extension, with unrestricted
//! network and process access, outside every Fyx check. The registry refuses one: the CI when it
//! signs or verifies a release, and the client when it downloads it. A local install only warns.

/// The file name a Zed extension's component has.
pub const ZED_EXTENSION_WASM: &str = "extension.wasm";

/// Every file name the zip archive `package` names, in its local file headers and its central
/// directory, as written. It reads only the headers, never the data, and over-reports: any byte
/// run that looks like a header counts, so a name hidden from one of the two lists (which an
/// unpacker might read instead of the other) is still found. Bytes that are not a zip name
/// nothing.
pub fn entry_names(package: &[u8]) -> Vec<String> {
    const LOCAL: &[u8; 4] = b"PK\x03\x04";
    const CENTRAL: &[u8; 4] = b"PK\x01\x02";
    let u16_at = |at: usize| -> Option<usize> {
        let bytes = package.get(at..at + 2)?;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
    };
    let mut names = Vec::new();
    for at in 0..package.len().saturating_sub(3) {
        let signature = &package[at..at + 4];
        // (offset of the name length, offset of the name) from the signature.
        let (length_at, name_at) = if signature == LOCAL {
            (26, 30)
        } else if signature == CENTRAL {
            (28, 46)
        } else {
            continue;
        };
        let Some(length) = u16_at(at + length_at) else {
            continue;
        };
        if let Some(name) = package.get(at + name_at..at + name_at + length) {
            names.push(String::from_utf8_lossy(name).into_owned());
        }
    }
    names
}

/// Whether `name` (an archive path) is a Zed `extension.wasm` at any depth, the way a
/// case-insensitive file system would see it (`EXTENSION.WASM`, `extension.wasm.`, `a\extension.wasm`).
pub fn is_zed_extension_wasm(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.trim_end_matches(['.', ' '])
        .eq_ignore_ascii_case(ZED_EXTENSION_WASM)
}

/// The package refused, if it contains a Zed `extension.wasm`.
pub fn check_package_contents(package: &[u8]) -> Result<(), String> {
    match entry_names(package)
        .into_iter()
        .find(|name| is_zed_extension_wasm(name))
    {
        Some(name) => Err(format!(
            "the package contains {name:?}, a Zed extension component: it would run with \
             unrestricted network and process access, so the registry does not accept it"
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored (uncompressed) zip with these files, written by hand: local headers, the
    /// central directory and its end record.
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = out.len() as u32;
            let header = |signature: &[u8], central: bool| {
                let mut header = signature.to_vec();
                if central {
                    header.extend_from_slice(&20u16.to_le_bytes()); // made by
                }
                header.extend_from_slice(&20u16.to_le_bytes()); // needed
                header.extend_from_slice(&[0; 2]); // flags
                header.extend_from_slice(&[0; 2]); // stored
                header.extend_from_slice(&[0; 4]); // time, date
                header.extend_from_slice(&[0; 4]); // crc (not checked here)
                header.extend_from_slice(&(data.len() as u32).to_le_bytes());
                header.extend_from_slice(&(data.len() as u32).to_le_bytes());
                header.extend_from_slice(&(name.len() as u16).to_le_bytes());
                header.extend_from_slice(&[0; 2]); // extra
                if central {
                    header.extend_from_slice(&[0; 2]); // comment
                    header.extend_from_slice(&[0; 2]); // disk
                    header.extend_from_slice(&[0; 2]); // internal attributes
                    header.extend_from_slice(&[0; 4]); // external attributes
                    header.extend_from_slice(&offset.to_le_bytes());
                }
                header.extend_from_slice(name.as_bytes());
                header
            };
            out.extend(header(b"PK\x03\x04", false));
            out.extend_from_slice(data);
            central.extend(header(b"PK\x01\x02", true));
        }
        let central_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out
    }

    #[test]
    fn a_fyx_package_passes() {
        let package = zip(&[
            ("extension.toml", b"id = \"todo\""),
            ("fyx_ui.wasm", b"\0asm"),
            ("assets/logo.svg", b"<svg/>"),
        ]);
        assert_eq!(
            entry_names(&package),
            [
                "extension.toml",
                "fyx_ui.wasm",
                "assets/logo.svg",
                "extension.toml",
                "fyx_ui.wasm",
                "assets/logo.svg"
            ]
        );
        assert_eq!(check_package_contents(&package), Ok(()));
        // Not a zip: names nothing (the download's hash check is what fails it).
        assert_eq!(check_package_contents(b"package"), Ok(()));
    }

    #[test]
    fn a_zed_extension_component_is_refused_at_any_depth_and_case() {
        for name in [
            "extension.wasm",
            "EXTENSION.WASM",
            "extension.wasm.",
            "lib/extension.wasm",
            "lib\\Extension.Wasm",
        ] {
            let package = zip(&[("extension.toml", b""), (name, b"\0asm")]);
            let error = check_package_contents(&package).unwrap_err();
            assert!(error.contains("Zed extension component"), "{name}: {error}");
        }
        assert!(!is_zed_extension_wasm("extension.wasm.map"));
        assert!(!is_zed_extension_wasm("my-extension.wasm"));
    }

    /// A name only in the local headers (an unpacker that streams them would still write it) is
    /// found, and so is one only in the central directory.
    #[test]
    fn a_name_hidden_from_one_list_is_found() {
        let mut package = zip(&[("extension.toml", b""), ("extension.wasm", b"\0asm")]);
        let central = package
            .windows(4)
            .rposition(|window| window == b"PK\x01\x02")
            .expect("a central entry");
        // Rename the central directory's entry: only the local header still says it.
        package[central + 46..central + 46 + 14].copy_from_slice(b"harmless.bin\0\0");
        assert!(check_package_contents(&package).is_err());

        let mut package = zip(&[("extension.toml", b""), ("extension.wasm", b"\0asm")]);
        let local = package
            .windows(4)
            .rposition(|window| window == b"PK\x03\x04")
            .expect("a local header");
        package[local + 30..local + 30 + 14].copy_from_slice(b"harmless.bin\0\0");
        assert!(check_package_contents(&package).is_err());
    }
}
