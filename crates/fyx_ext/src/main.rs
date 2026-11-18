use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use fyx_ext::CheckDepth;
use fyx_ext::build::{BuildOptions, DevOptions};
use fyx_ext::template::{GitReference, NewExtension, SdkSource};

/// Create, build, check, run and package Fyx UI extensions.
#[derive(Parser)]
#[command(name = "fyx-ext", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new Rust extension crate. Fyx extensions are Rust only until 1.0 (platform spec,
    /// decision 3).
    New {
        /// Crate name, such as my_panel or my-panel. The extension id uses `-`.
        name: String,
        /// Parent directory of the new crate.
        #[arg(long, default_value = ".")]
        out: PathBuf,
        /// Depend on the SDK in this Fyx checkout (default: the checkout fyx-ext was built from).
        #[arg(long, conflicts_with = "sdk_git")]
        sdk_path: Option<PathBuf>,
        /// Depend on the SDK from this git repository instead.
        #[arg(long)]
        sdk_git: Option<String>,
        /// Branch of --sdk-git (default: fyx/main, the repository's main branch).
        #[arg(long, requires = "sdk_git", conflicts_with = "sdk_rev")]
        sdk_branch: Option<String>,
        /// Commit of --sdk-git.
        #[arg(long, requires = "sdk_git")]
        sdk_rev: Option<String>,
    },
    /// Build for wasm32-wasip2 and copy the component next to extension.toml.
    Build {
        /// The extension directory (with extension.toml).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Build with the release profile.
        #[arg(long)]
        release: bool,
        /// Arguments for cargo build, after `--`.
        #[arg(last = true)]
        cargo_args: Vec<String>,
    },
    /// Build, start Fyx with the extension loaded, and rebuild on every change.
    Dev {
        /// The extension directory (with extension.toml).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// The Fyx binary (default: FYX_BIN, then `fyx` on PATH).
        #[arg(long)]
        fyx: Option<PathBuf>,
        /// Only build and watch; print how to start Fyx.
        #[arg(long)]
        no_launch: bool,
        /// Build once and return.
        #[arg(long)]
        no_watch: bool,
        /// Arguments for Fyx, after `--`.
        #[arg(last = true)]
        fyx_args: Vec<String>,
    },
    /// Check extension.toml and the built component the way Fyx does when it loads them.
    Check {
        /// The extension directory (with extension.toml).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Fail on warnings too.
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Build, check, run every surface headless, then the crate's own `cargo test` with
    /// FYX_EXT_COMPONENT set to the built component.
    Test {
        /// The extension directory (with extension.toml).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Arguments for cargo test, after `--`.
        #[arg(last = true)]
        cargo_args: Vec<String>,
    },
    /// Build for release, check, and write <id>-<version>.fyxext.
    Package {
        /// The extension directory (with extension.toml).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Where the package goes (default: the extension directory).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Package the component as it is, without building.
        #[arg(long)]
        no_build: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::New {
            name,
            out,
            sdk_path,
            sdk_git,
            sdk_branch,
            sdk_rev,
        } => {
            let sdk = match sdk_git {
                Some(url) => SdkSource::Git {
                    url,
                    reference: match sdk_rev {
                        Some(rev) => GitReference::Rev(rev),
                        None => GitReference::Branch(
                            sdk_branch.unwrap_or_else(|| "fyx/main".to_string()),
                        ),
                    },
                },
                None => SdkSource::Checkout(
                    sdk_path.unwrap_or_else(fyx_ext::template::default_fyx_checkout),
                ),
            };
            let crate_dir = fyx_ext::template::create(&NewExtension {
                name,
                parent: out,
                sdk,
            })?;
            eprintln!("fyx-ext: created {}", crate_dir.display());
            eprintln!(
                "Next:\n  cd {}\n  fyx-ext dev --fyx <path to the fyx binary>\n(If fyx-ext is not on PATH: RUSTC_WRAPPER= cargo install --path <fyx checkout>/crates/fyx_ext)",
                crate_dir.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Build {
            path,
            release,
            cargo_args,
        } => {
            fyx_ext::build::build(
                &path,
                &BuildOptions {
                    release,
                    cargo_args,
                },
            )?;
            let (_, report) = fyx_ext::check(&path, CheckDepth::Load)?;
            report.print();
            Ok(exit_code(report.has_errors()))
        }
        Command::Dev {
            path,
            fyx,
            no_launch,
            no_watch,
            fyx_args,
        } => {
            fyx_ext::build::dev(
                &path,
                &DevOptions {
                    build: BuildOptions::default(),
                    fyx,
                    launch: !no_launch,
                    watch: !no_watch,
                    fyx_args,
                },
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            path,
            deny_warnings,
        } => {
            let (manifest, report) = fyx_ext::check(&path, CheckDepth::Load)?;
            report.print();
            let failed = report.has_errors() || (deny_warnings && !report.warnings.is_empty());
            if !failed {
                eprintln!("fyx-ext: {} {} passes", manifest.id, manifest.version);
            }
            Ok(exit_code(failed))
        }
        Command::Test { path, cargo_args } => {
            let passed = fyx_ext::run::test(&path, &cargo_args)?;
            Ok(exit_code(!passed))
        }
        Command::Package {
            path,
            out,
            no_build,
        } => {
            if !no_build {
                fyx_ext::build::build(
                    &path,
                    &BuildOptions {
                        release: true,
                        cargo_args: Vec::new(),
                    },
                )?;
            }
            let out = out.unwrap_or_else(|| path.clone());
            fyx_ext::package::package(&path, &out, CheckDepth::Load)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn exit_code(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
