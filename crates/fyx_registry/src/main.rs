//! `fyx-registry`: the registry CI's signing and index tool (plan task 3.2).
//!
//! The signing key comes from `--key-file` or the `FYX_REGISTRY_SIGNING_KEY` environment
//! variable, as `{"key_id": "...", "seed": "<base64 of 32 bytes>"}`. This tool never creates or
//! writes a signing key. The variable is removed from the environment as soon as it is read, and
//! on Unix a key file readable by group or others is refused (security-2 T9).

use std::{
    collections::BTreeSet,
    env::VarError,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use fyx_registry::{
    Blocklist, Index, KeyId, KeyRole, Keyset, Release, Signed, SigningKey, Snapshot, Timestamp,
    VerifiedBlocklist, VerifiedSnapshot,
};
use zeroize::Zeroizing;

const SIGNING_KEY_VARIABLE: &str = "FYX_REGISTRY_SIGNING_KEY";

#[derive(Parser)]
#[command(
    name = "fyx-registry",
    about = "Sign and verify Fyx registry documents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign a release or a block list.
    Sign {
        #[command(subcommand)]
        document: SignDocument,
    },
    /// Verify signatures and contents.
    Verify {
        #[command(subcommand)]
        document: VerifyDocument,
    },
    /// Edit index.json.
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Print the public key entry for the signing key, for a keyset file.
    PublicKey {
        /// What the key may sign: release, blocklist, snapshot. At least one.
        #[arg(long = "role", required = true, value_parser = parse_role)]
        roles: Vec<KeyRole>,
        #[command(flatten)]
        key: KeyArgs,
    },
}

#[derive(Subcommand)]
enum SignDocument {
    /// Sign an unsigned release (the `signed` part of an index entry).
    Release {
        file: PathBuf,
        /// The package the release names; its size and sha256 are checked before signing.
        #[arg(long)]
        package: PathBuf,
        #[command(flatten)]
        key: KeyArgs,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Sign an unsigned block list.
    Blocklist {
        file: PathBuf,
        #[command(flatten)]
        key: KeyArgs,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Sign the snapshot of an index: its size and sha256, the time, an expiry and a sequence.
    Snapshot {
        #[arg(long)]
        index: PathBuf,
        /// Must be higher than every earlier snapshot's.
        #[arg(long)]
        sequence: u64,
        /// How long the snapshot is valid; the CI re-signs before it expires.
        #[arg(long, default_value_t = 48)]
        validity_hours: u64,
        /// Unix seconds for `generated_at`; defaults to now.
        #[arg(long)]
        generated_at: Option<u64>,
        /// A key whose signatures clients refuse from now on. Repeat for several.
        #[arg(long = "revoke")]
        revoked_keys: Vec<String>,
        #[command(flatten)]
        key: KeyArgs,
        #[command(flatten)]
        output: OutputArgs,
    },
}

#[derive(Subcommand)]
enum VerifyDocument {
    /// Verify a signed release, and optionally its package file.
    Release {
        file: PathBuf,
        #[arg(long)]
        keyset: PathBuf,
        #[arg(long)]
        package: Option<PathBuf>,
    },
    /// Verify every release in an index, and with `--snapshot` that the snapshot names it.
    Index {
        file: PathBuf,
        #[arg(long)]
        keyset: PathBuf,
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Verify a signed snapshot and that it has not expired.
    Snapshot {
        file: PathBuf,
        #[arg(long)]
        keyset: PathBuf,
    },
    /// Verify a signed block list.
    Blocklist {
        file: PathBuf,
        #[arg(long)]
        keyset: PathBuf,
    },
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Add a signed release to an index, creating the index if it does not exist.
    Add {
        #[arg(long)]
        index: PathBuf,
        #[arg(long)]
        release: PathBuf,
        #[arg(long)]
        keyset: PathBuf,
        /// Unix seconds for `generated_at`; defaults to now.
        #[arg(long)]
        generated_at: Option<u64>,
    },
}

#[derive(Args)]
struct KeyArgs {
    /// Signing key file; without it the key is read from FYX_REGISTRY_SIGNING_KEY.
    #[arg(long)]
    key_file: Option<PathBuf>,
}

#[derive(Args)]
struct OutputArgs {
    /// Write here instead of standard output.
    #[arg(long)]
    out: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let signing_key_variable = match std::env::var(SIGNING_KEY_VARIABLE) {
        Ok(value) => Some(Zeroizing::new(value)),
        Err(VarError::NotPresent) => None,
        Err(VarError::NotUnicode(_)) => {
            eprintln!("fyx-registry: {SIGNING_KEY_VARIABLE} is not valid UTF-8");
            return ExitCode::FAILURE;
        }
    };
    // SAFETY: nothing else runs yet: no thread has been started and no library has read the
    // environment, so removing a variable cannot race with a reader.
    unsafe { std::env::remove_var(SIGNING_KEY_VARIABLE) };
    let mut stdout = io::stdout().lock();
    match run(cli, signing_key_variable, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fyx-registry: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    cli: Cli,
    signing_key_variable: Option<Zeroizing<String>>,
    stdout: &mut dyn Write,
) -> Result<()> {
    match cli.command {
        Command::Sign { document } => match document {
            SignDocument::Release {
                file,
                package,
                key,
                output,
            } => {
                let key = signing_key(&key, signing_key_variable)?;
                let release: Release = serde_json::from_slice(&read(&file)?)
                    .with_context(|| format!("{} is not an unsigned release", file.display()))?;
                release.validate()?;
                release.verify_package(&read(&package)?)?;
                let signed = Signed::sign(release, &key);
                write_output(&output, &to_json(&signed)?, stdout)
            }
            SignDocument::Blocklist { file, key, output } => {
                let key = signing_key(&key, signing_key_variable)?;
                let blocklist: Blocklist = serde_json::from_slice(&read(&file)?)
                    .with_context(|| format!("{} is not an unsigned block list", file.display()))?;
                blocklist.validate()?;
                let signed = Signed::sign(blocklist, &key);
                write_output(&output, &to_json(&signed)?, stdout)
            }
            SignDocument::Snapshot {
                index,
                sequence,
                validity_hours,
                generated_at,
                revoked_keys,
                key,
                output,
            } => {
                let key = signing_key(&key, signing_key_variable)?;
                let index_bytes = read(&index)?;
                Index::from_json(&index_bytes)
                    .with_context(|| format!("{} is not an index", index.display()))?;
                let generated_at = match generated_at {
                    Some(seconds) => Timestamp(seconds),
                    None => now()?,
                };
                let revoked_keys = revoked_keys
                    .into_iter()
                    .map(KeyId::new)
                    .collect::<Result<BTreeSet<_>, _>>()?;
                let snapshot = Snapshot::for_index(
                    &index_bytes,
                    sequence,
                    generated_at,
                    Duration::from_secs(validity_hours.saturating_mul(3600)),
                    revoked_keys,
                );
                snapshot.validate()?;
                let signed = Signed::sign(snapshot, &key);
                write_output(&output, &Snapshot::to_json(&signed)?, stdout)
            }
        },
        Command::Verify { document } => match document {
            VerifyDocument::Release {
                file,
                keyset,
                package,
            } => {
                let keyset = read_keyset(&keyset)?;
                let signed = Release::from_json(&read(&file)?)?;
                let release = signed.verify_release(&keyset)?;
                if let Some(package) = package {
                    release.verify_package(&read(&package)?)?;
                }
                writeln!(
                    stdout,
                    "ok: {}@{} signed by {}",
                    release.id, release.version, signed.signature.key_id
                )?;
                Ok(())
            }
            VerifyDocument::Index {
                file,
                keyset,
                snapshot,
            } => {
                let keyset = read_keyset(&keyset)?;
                let bytes = read(&file)?;
                let index = Index::from_json(&bytes)?;
                let verified = match snapshot {
                    Some(snapshot) => {
                        let snapshot =
                            VerifiedSnapshot::from_json(&read(&snapshot)?, &keyset, now()?, None)?;
                        Index::verify_with_snapshot(&bytes, &snapshot, &keyset)?
                    }
                    None => index.verify(&keyset),
                };
                for rejected in verified.rejected() {
                    writeln!(stdout, "rejected: {}: {}", rejected.label(), rejected.error)?;
                }
                let entries = index.releases.len() + index.malformed.len();
                if !verified.rejected().is_empty() {
                    bail!(
                        "{} of {entries} releases in {} do not verify",
                        verified.rejected().len(),
                        file.display()
                    );
                }
                index.verify_strict(&keyset)?;
                writeln!(stdout, "ok: {entries} releases")?;
                Ok(())
            }
            VerifyDocument::Snapshot { file, keyset } => {
                let keyset = read_keyset(&keyset)?;
                let snapshot = VerifiedSnapshot::from_json(&read(&file)?, &keyset, now()?, None)?;
                writeln!(
                    stdout,
                    "ok: snapshot sequence {} expires at {} signed by {}",
                    snapshot.sequence(),
                    snapshot.expires_at().0,
                    snapshot.signed().signature.key_id
                )?;
                Ok(())
            }
            VerifyDocument::Blocklist { file, keyset } => {
                let keyset = read_keyset(&keyset)?;
                let blocklist = VerifiedBlocklist::from_json(&read(&file)?, &keyset)?;
                writeln!(
                    stdout,
                    "ok: block list sequence {} with {} entries signed by {}",
                    blocklist.sequence(),
                    blocklist.entries().len(),
                    blocklist.signed().signature.key_id
                )?;
                Ok(())
            }
        },
        Command::Index {
            command:
                IndexCommand::Add {
                    index: index_path,
                    release,
                    keyset,
                    generated_at,
                },
        } => {
            let keyset = read_keyset(&keyset)?;
            let generated_at = match generated_at {
                Some(seconds) => Timestamp(seconds),
                None => now()?,
            };
            let mut index = if index_path.exists() {
                Index::from_json(&read(&index_path)?)?
            } else {
                Index::new(generated_at)
            };
            let signed = Release::from_json(&read(&release)?)?;
            let added = format!("{}@{}", signed.signed.id, signed.signed.version);
            index.add(signed, &keyset, generated_at)?;
            write_atomically(&index_path, &index.to_json()?)?;
            writeln!(
                stdout,
                "added {added}; {} has {} releases",
                index_path.display(),
                index.releases.len()
            )?;
            Ok(())
        }
        Command::PublicKey { roles, key } => {
            let key = signing_key(&key, signing_key_variable)?;
            stdout.write_all(&to_json(&key.public_key(roles))?)?;
            Ok(())
        }
    }
}

fn parse_role(text: &str) -> Result<KeyRole, String> {
    KeyRole::ALL
        .into_iter()
        .find(|role| role.as_str() == text)
        .ok_or_else(|| format!("{text:?} is not a role: use release, blocklist or snapshot"))
}

/// On Unix a signing key file must not be readable by group or others.
fn check_key_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(path)
            .with_context(|| format!("reading {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            bail!(
                "signing key file {} is readable by others (mode {:o}); run chmod 600 on it",
                path.display(),
                mode & 0o777
            );
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn signing_key(key: &KeyArgs, variable: Option<Zeroizing<String>>) -> Result<SigningKey> {
    let contents = match (&key.key_file, variable) {
        (Some(path), _) => {
            check_key_file_permissions(path)?;
            Zeroizing::new(read(path)?)
        }
        (None, Some(variable)) => Zeroizing::new(variable.as_bytes().to_vec()),
        (None, None) => bail!("no signing key: pass --key-file or set {SIGNING_KEY_VARIABLE}"),
    };
    Ok(SigningKey::from_json(&contents)?)
}

fn read_keyset(path: &Path) -> Result<Keyset> {
    Keyset::from_json(&read(path)?).with_context(|| format!("reading keyset {}", path.display()))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_output(output: &OutputArgs, bytes: &[u8], stdout: &mut dyn Write) -> Result<()> {
    match &output.out {
        Some(path) => write_atomically(path, bytes),
        None => Ok(stdout.write_all(bytes)?),
    }
}

/// A CI job killed mid-write must not leave a truncated index behind. The temporary file has a
/// random name and is created exclusively, so a link planted at a predictable name cannot
/// redirect the write (security-2 T9).
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    use ring::rand::{SecureRandom as _, SystemRandom};

    let mut suffix = [0u8; 8];
    SystemRandom::new()
        .fill(&mut suffix)
        .map_err(|_| anyhow::anyhow!("no random numbers for a temporary file name"))?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", hex::encode(suffix)));
    let temporary = PathBuf::from(temporary);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written.and_then(|()| fs::rename(&temporary, path)) {
        fs::remove_file(&temporary).ok();
        return Err(error).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

fn now() -> Result<Timestamp> {
    let elapsed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("the system clock is before 1970")?;
    Ok(Timestamp(elapsed.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use fyx_registry::Sha256Digest;
    use serde_json::json;

    // A test-only seed; it is public in this file and must never sign real releases.
    fn test_key_json(key_id: &str, seed_byte: u8) -> String {
        json!({"key_id": key_id, "seed": BASE64.encode([seed_byte; 32])}).to_string()
    }

    struct Fixture {
        directory: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let fixture = Self {
                directory: tempfile::tempdir().expect("temporary directory"),
            };
            fixture.write("key.json", test_key_json("test-registry", 1).as_bytes());
            let public_key = SigningKey::from_json(test_key_json("test-registry", 1).as_bytes())
                .expect("test key")
                .public_key(KeyRole::ALL);
            let keyset = json!({"keys": [public_key]});
            fixture.write("keyset.json", keyset.to_string().as_bytes());
            fixture.write("todo-1.0.0.fyxext", b"package bytes");
            fixture
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.path().join(name)
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            fs::write(self.path(name), bytes).expect("writes");
        }

        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.path(name)).expect("reads")
        }

        fn release_json(&self, version: &str) -> String {
            let package = b"package bytes";
            json!({
                "id": "todo",
                "version": version,
                "name": "Todo",
                "description": "Keeps a list of things to do.",
                "owners": ["alice"],
                "source": {
                    "repository": "https://github.com/alice/todo",
                    "commit": "0123456789abcdef0123456789abcdef01234567",
                    "path": "."
                },
                "package": {
                    "url": format!("https://example.invalid/todo-{version}.fyxext"),
                    "sha256": Sha256Digest::of(package).to_string(),
                    "size": package.len()
                },
                "published_at": 1_000_000,
                "urgent": false,
                "tier": "t0",
                "capabilities": ["ui:panel"]
            })
            .to_string()
        }

        fn run(&self, arguments: &[&str]) -> Result<String> {
            self.run_with_variable(arguments, None)
        }

        fn run_with_variable(
            &self,
            arguments: &[&str],
            variable: Option<String>,
        ) -> Result<String> {
            let directory = self.directory.path().to_string_lossy().into_owned();
            let arguments = arguments
                .iter()
                .map(|argument| argument.replace("{dir}", &directory));
            let cli =
                Cli::try_parse_from(std::iter::once("fyx-registry".to_string()).chain(arguments))?;
            let mut output = Vec::new();
            run(cli, variable.map(Zeroizing::new), &mut output)?;
            Ok(String::from_utf8(output)?)
        }
    }

    #[test]
    fn sign_verify_and_add_releases() {
        let fixture = Fixture::new();
        fixture.write("release.json", fixture.release_json("1.0.0").as_bytes());
        fixture
            .run(&[
                "sign",
                "release",
                "{dir}/release.json",
                "--package",
                "{dir}/todo-1.0.0.fyxext",
                "--key-file",
                "{dir}/key.json",
                "--out",
                "{dir}/signed.json",
            ])
            .expect("signs");
        let verified = fixture
            .run(&[
                "verify",
                "release",
                "{dir}/signed.json",
                "--keyset",
                "{dir}/keyset.json",
                "--package",
                "{dir}/todo-1.0.0.fyxext",
            ])
            .expect("verifies");
        assert_eq!(verified, "ok: todo@1.0.0 signed by test-registry\n");

        let added = fixture
            .run(&[
                "index",
                "add",
                "--index",
                "{dir}/index.json",
                "--release",
                "{dir}/signed.json",
                "--keyset",
                "{dir}/keyset.json",
                "--generated-at",
                "5",
            ])
            .expect("adds");
        assert!(added.starts_with("added todo@1.0.0;"), "{added}");
        let index = Index::from_json(fixture.read("index.json").as_bytes()).expect("parses");
        assert_eq!(index.generated_at, Timestamp(5));
        assert_eq!(index.releases.len(), 1);

        let error = fixture
            .run(&[
                "index",
                "add",
                "--index",
                "{dir}/index.json",
                "--release",
                "{dir}/signed.json",
                "--keyset",
                "{dir}/keyset.json",
            ])
            .expect_err("a published version is never replaced");
        assert!(
            error.to_string().contains("already in the index"),
            "{error}"
        );

        let verified = fixture
            .run(&[
                "verify",
                "index",
                "{dir}/index.json",
                "--keyset",
                "{dir}/keyset.json",
            ])
            .expect("verifies");
        assert_eq!(verified, "ok: 1 releases\n");

        // The snapshot pins this index; once the index changes it no longer verifies.
        let signed = fixture
            .run(&[
                "sign",
                "snapshot",
                "--index",
                "{dir}/index.json",
                "--sequence",
                "1",
                "--key-file",
                "{dir}/key.json",
                "--out",
                "{dir}/snapshot.json",
            ])
            .expect("signs the snapshot");
        assert!(signed.is_empty());
        let verified = fixture
            .run(&[
                "verify",
                "snapshot",
                "{dir}/snapshot.json",
                "--keyset",
                "{dir}/keyset.json",
            ])
            .expect("verifies the snapshot");
        assert!(
            verified.starts_with("ok: snapshot sequence 1"),
            "{verified}"
        );
        let with_snapshot = [
            "verify",
            "index",
            "{dir}/index.json",
            "--keyset",
            "{dir}/keyset.json",
            "--snapshot",
            "{dir}/snapshot.json",
        ];
        fixture.run(&with_snapshot).expect("matches");
        let edited = fixture
            .read("index.json")
            .replace("\"generated_at\": 5", "\"generated_at\": 6");
        fixture.write("index.json", edited.as_bytes());
        let error = fixture.run(&with_snapshot).expect_err("another index");
        assert!(
            error
                .to_string()
                .contains("is not the index snapshot 1 names"),
            "{error}"
        );
    }

    #[test]
    fn tampering_is_refused_everywhere() {
        let fixture = Fixture::new();
        fixture.write("release.json", fixture.release_json("1.0.0").as_bytes());
        assert!(
            fixture
                .run(&[
                    "sign",
                    "release",
                    "{dir}/release.json",
                    "--key-file",
                    "{dir}/key.json",
                ])
                .is_err(),
            "--package is required"
        );
        fixture
            .run(&[
                "sign",
                "release",
                "{dir}/release.json",
                "--package",
                "{dir}/todo-1.0.0.fyxext",
                "--key-file",
                "{dir}/key.json",
                "--out",
                "{dir}/signed.json",
            ])
            .expect("signs");
        fixture
            .run(&[
                "index",
                "add",
                "--index",
                "{dir}/index.json",
                "--release",
                "{dir}/signed.json",
                "--keyset",
                "{dir}/keyset.json",
            ])
            .expect("adds");

        let tampered = fixture
            .read("signed.json")
            .replace("\"urgent\": false", "\"urgent\": true");
        fixture.write("tampered.json", tampered.as_bytes());
        assert!(
            fixture
                .run(&[
                    "verify",
                    "release",
                    "{dir}/tampered.json",
                    "--keyset",
                    "{dir}/keyset.json"
                ])
                .is_err()
        );
        assert!(
            fixture
                .run(&[
                    "index",
                    "add",
                    "--index",
                    "{dir}/other-index.json",
                    "--release",
                    "{dir}/tampered.json",
                    "--keyset",
                    "{dir}/keyset.json",
                ])
                .is_err()
        );
        assert!(!fixture.path("other-index.json").exists());

        let tampered_index = fixture.read("index.json").replace("alice", "mallory");
        fixture.write("index.json", tampered_index.as_bytes());
        let error = fixture
            .run(&[
                "verify",
                "index",
                "{dir}/index.json",
                "--keyset",
                "{dir}/keyset.json",
            ])
            .expect_err("tampered index");
        assert!(error.to_string().contains("1 of 1 releases"), "{error}");

        fixture.write("other.fyxext", b"other bytes!!");
        assert!(
            fixture
                .run(&[
                    "verify",
                    "release",
                    "{dir}/signed.json",
                    "--keyset",
                    "{dir}/keyset.json",
                    "--package",
                    "{dir}/other.fyxext",
                ])
                .is_err()
        );
        assert!(
            fixture
                .run(&[
                    "sign",
                    "release",
                    "{dir}/release.json",
                    "--key-file",
                    "{dir}/key.json",
                    "--package",
                    "{dir}/other.fyxext",
                ])
                .is_err()
        );
    }

    #[test]
    fn signs_block_lists_with_the_key_from_the_environment() {
        let fixture = Fixture::new();
        let blocklist = json!({
            "schema": 1,
            "sequence": 7,
            "generated_at": 100,
            "expires_at": 100 + 86_400,
            "entries": [
                {"id": "todo", "versions": ">=1.2.0, <1.3.0", "reason": "Leaks buffers.", "urgent": true}
            ]
        });
        fixture.write("blocklist-unsigned.json", blocklist.to_string().as_bytes());
        let signed = fixture
            .run_with_variable(
                &["sign", "blocklist", "{dir}/blocklist-unsigned.json"],
                Some(test_key_json("test-registry", 1)),
            )
            .expect("signs");
        fixture.write("blocklist.json", signed.as_bytes());
        let verified = fixture
            .run(&[
                "verify",
                "blocklist",
                "{dir}/blocklist.json",
                "--keyset",
                "{dir}/keyset.json",
            ])
            .expect("verifies");
        assert_eq!(
            verified,
            "ok: block list sequence 7 with 1 entries signed by test-registry\n"
        );

        let other_key = fixture
            .run_with_variable(
                &["sign", "blocklist", "{dir}/blocklist-unsigned.json"],
                Some(test_key_json("test-other", 2)),
            )
            .expect("signs");
        fixture.write("blocklist.json", other_key.as_bytes());
        assert!(
            fixture
                .run(&[
                    "verify",
                    "blocklist",
                    "{dir}/blocklist.json",
                    "--keyset",
                    "{dir}/keyset.json"
                ])
                .is_err()
        );
    }

    #[test]
    fn needs_a_key_and_prints_the_public_key() {
        let fixture = Fixture::new();
        fixture.write("release.json", fixture.release_json("1.0.0").as_bytes());
        let error = fixture
            .run(&[
                "sign",
                "release",
                "{dir}/release.json",
                "--package",
                "{dir}/todo-1.0.0.fyxext",
            ])
            .expect_err("no key");
        assert!(error.to_string().contains(SIGNING_KEY_VARIABLE), "{error}");

        assert!(
            fixture
                .run(&["public-key", "--key-file", "{dir}/key.json"])
                .is_err(),
            "a role is required"
        );
        let printed = fixture
            .run(&[
                "public-key",
                "--key-file",
                "{dir}/key.json",
                "--role",
                "release",
                "--role",
                "snapshot",
            ])
            .expect("prints");
        let public_key: fyx_registry::PublicKey =
            serde_json::from_str(&printed).expect("a keyset entry");
        assert_eq!(
            public_key.key_id,
            KeyId::new("test-registry").expect("valid")
        );
        assert_eq!(
            public_key.roles,
            [KeyRole::Release, KeyRole::Snapshot].into()
        );
        assert!(!printed.contains(&BASE64.encode([1u8; 32])));
    }

    #[test]
    fn atomic_writes_leave_no_temporary_file() {
        let fixture = Fixture::new();
        write_atomically(&fixture.path("out.json"), b"one").expect("writes");
        write_atomically(&fixture.path("out.json"), b"two").expect("replaces");
        assert_eq!(fixture.read("out.json"), "two");
        let names: Vec<String> = fs::read_dir(fixture.directory.path())
            .expect("lists")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            !names.iter().any(|name| name.ends_with(".tmp")),
            "{names:?}"
        );
    }
}
