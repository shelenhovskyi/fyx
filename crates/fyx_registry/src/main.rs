//! `fyx-registry`: the registry CI's signing and index tool (plan task 3.2).
//!
//! The signing key comes from `--key-file` or the `FYX_REGISTRY_SIGNING_KEY` environment
//! variable, as `{"key_id": "...", "seed": "<base64 of 32 bytes>"}`. This tool never creates or
//! writes a signing key.

use std::{
    env::VarError,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::SystemTime,
};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use fyx_registry::{
    Blocklist, Index, Keyset, Release, Signed, SigningKey, Timestamp, VerifiedBlocklist,
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
        #[command(flatten)]
        key: KeyArgs,
    },
}

#[derive(Subcommand)]
enum SignDocument {
    /// Sign an unsigned release (the `signed` part of an index entry).
    Release {
        file: PathBuf,
        /// Check the release's package size and sha256 against this file before signing.
        #[arg(long)]
        package: Option<PathBuf>,
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
    /// Verify every release in an index.
    Index {
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
                if let Some(package) = package {
                    release.verify_package(&read(&package)?)?;
                }
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
            VerifyDocument::Index { file, keyset } => {
                let keyset = read_keyset(&keyset)?;
                let index = Index::from_json(&read(&file)?)?;
                let verified = index.verify(&keyset);
                for rejected in verified.rejected() {
                    writeln!(
                        stdout,
                        "rejected: {}@{}: {}",
                        rejected.id, rejected.version, rejected.error
                    )?;
                }
                if !verified.rejected().is_empty() {
                    bail!(
                        "{} of {} releases in {} do not verify",
                        verified.rejected().len(),
                        index.releases.len(),
                        file.display()
                    );
                }
                writeln!(stdout, "ok: {} releases", index.releases.len())?;
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
        Command::PublicKey { key } => {
            let key = signing_key(&key, signing_key_variable)?;
            stdout.write_all(&to_json(&key.public_key())?)?;
            Ok(())
        }
    }
}

fn signing_key(key: &KeyArgs, variable: Option<Zeroizing<String>>) -> Result<SigningKey> {
    let contents = match (&key.key_file, variable) {
        (Some(path), _) => Zeroizing::new(read(path)?),
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

/// A CI job killed mid-write must not leave a truncated index behind.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes).with_context(|| format!("writing {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))
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
    use fyx_registry::{KeyId, Sha256Digest};
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
                .public_key();
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
    }

    #[test]
    fn tampering_is_refused_everywhere() {
        let fixture = Fixture::new();
        fixture.write("release.json", fixture.release_json("1.0.0").as_bytes());
        fixture
            .run(&[
                "sign",
                "release",
                "{dir}/release.json",
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
            .run(&["sign", "release", "{dir}/release.json"])
            .expect_err("no key");
        assert!(error.to_string().contains(SIGNING_KEY_VARIABLE), "{error}");

        let printed = fixture
            .run(&["public-key", "--key-file", "{dir}/key.json"])
            .expect("prints");
        let public_key: fyx_registry::PublicKey =
            serde_json::from_str(&printed).expect("a keyset entry");
        assert_eq!(
            public_key.key_id,
            KeyId::new("test-registry").expect("valid")
        );
        assert!(!printed.contains(&BASE64.encode([1u8; 32])));
    }
}
