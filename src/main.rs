#![warn(rust_2018_idioms, rustdoc::broken_intra_doc_links)]
#![deny(elided_lifetimes_in_paths)]
#![forbid(unsafe_code)]

//! `cargo doc2readme` is a cargo subcommand to create a readme file to display on
//! [GitHub] or [crates.io],
//! containing the rustdoc comments from your code.
//!
//! # Installation
//!
//! If you are using ArchLinux, you can install cargo-doc2readme from the AUR:
//!
//! ```bash
//! yay -S cargo-doc2readme
//! ```
//!
//! On other Operating Systems, make sure you have Rust installed (using your
//! distributions package manager, but if your package manager is garbage or you are
//! running Windows, try [rustup](https://rustup.rs/)) and then run the following command:
//!
//! ```bash
//! cargo install cargo-doc2readme
//! ```
//!
//! # Usage
//!
//! To generate your readme, simply run
//!
//! ```bash
//! cargo doc2readme
//! ```
//!
//! This will output the readme to a file called `README.md`, using `README.j2` or the
//! built-in template.
//!
//! If you want to run this using GitHub Actions, you can use the pre-built docker image:
//!
//! ```yaml
//! readme:
//!   runs-on: ubuntu-latest
//!   steps:
//!     - uses: actions/checkout@v3
//!     - uses: docker://ghcr.io/msrd0/cargo-doc2readme
//!       with:
//!         entrypoint: cargo
//!         args: doc2readme --check
//! ```
//!
//! This will use the latest stable Rust version available when the latest release of
//! cargo doc2readme was created. If you need a newer/nightly Rust compiler, use the
//! `ghcr.io/msrd0/cargo-doc2readme:nightly` docker image instead.
//!
//! # Features
//!
//!  - parse markdown from your rustdoc comments and embed it into your readme
//!  - use existing crates to parse Rust and Markdown
//!  - support your `[CustomType]` rustdoc links
//!  - default, minimalistic readme template with some useful badges
//!  - custom readme templates
//!
//! # Non-Goals
//!
//!  - verbatim copy of your markdown
//!  - easy readability of the generated markdown source code
//!
//! # Similar tools
//!
//! [`cargo readme`][cargo-readme] is a similar tool. However, it brings its own Rust code
//! parser that only covers the 95% use case. Also, it does not support Rust path links
//! introduced in Rust 1.48, making your readme ugly due to GitHub showing the unsupported
//! links as raw markdown, and being less convenient for the reader that has to search
//! [docs.rs] instead of clicking on a link.
//!
//! # Stability Guarantees
//!
//! This project adheres to semantic versioning. All versions will be tested against the
//! latest stable rust version at the time of the release. All non-bugfix changes to the
//! rustdoc input processing and markdown output or the default readme template are
//! considered breaking changes, as well as any non-backwards-compatible changes to the
//! command-line arguments or to these stability guarantees. All other changes, including
//! any changes to the Rust code, or bumping the MSRV, are not considered breaking changes.
//!
//!  [crates.io]: https://crates.io
//!  [GitHub]: https://github.com
//!  [cargo-readme]: https://github.com/livioribeiro/cargo-readme
//!  [docs.rs]: https://docs.rs

use cargo_doc2readme::{diagnostic::Diagnostic, output, read_input, verify};
use clap::Parser;
use log::{error, info, warn, Level};
use std::{env, fs::File, io, path::PathBuf, process::ExitCode};

#[derive(Parser)]
enum Subcommand {
	Doc2readme(Args)
}

#[derive(Parser)]
#[command(about, version)]
struct Args {
	/// Path to Cargo.toml.
	#[arg(long)]
	manifest_path: Option<PathBuf>,

	/// Output File.
	#[arg(short, long, default_value = "README.md")]
	out: PathBuf,

	/// Template File. This is processed by minijinja. Look at the source code for
	/// cargo-doc2readme for an example.
	#[arg(short, long, default_value = "README.j2")]
	template: PathBuf,

	/// Use nightly rustc to expand macros prior to reading the source. This is necessary
	/// if you use function-like macros in doc attributes, as introduced in Rust 1.54.
	#[arg(long)]
	expand_macros: bool,

	/// Space or comma separated list of features to activate. This will be ignored unless
	/// `--expand-macros` is enabled, in which case it is being passed to cargo.
	#[arg(short = 'F', long)]
	features: Option<String>,

	/// Activate all available features. This will be ignored unless `--expand-macros` is
	/// enabled, in which case it is being passed to cargo.
	#[arg(long)]
	all_features: bool,

	/// Do not activate the `default` feature. This will be ignored unless
	/// `--expand-macros` is enabled, in which case it is being passed to cargo.
	#[arg(long)]
	no_default_features: bool,

	/// Prefer binary targets over library targets for rustdoc source.
	#[arg(long, conflicts_with = "lib")]
	bin: bool,

	/// Prefer library targets over binary targets for rustdoc source. This is the default.
	#[arg(long, conflicts_with = "bin")]
	lib: bool,

	/// Verify that the output file is (reasonably) up to date, and fail
	/// if it needs updating. The output file will not be changed.
	#[arg(long)]
	check: bool,

	/// Enable verbose output.
	#[arg(short, long)]
	verbose: bool
}

#[derive(Parser)]
struct CmdLine {
	#[clap(subcommand)]
	cmd: Subcommand
}

macro_rules! exit_on_err {
	($diagnostics:ident) => {
		if $diagnostics.is_fail() {
			return ExitCode::FAILURE;
		}
	};
}

fn main() -> ExitCode {
	let args = match env::args().nth(1) {
		Some(subcmd) if subcmd == "doc2readme" => match CmdLine::parse().cmd {
			Subcommand::Doc2readme(args) => args
		},
		_ => Args::parse()
	};

	// check input
	if !args.expand_macros {
		let mut diag = Diagnostic::new(String::new(), String::new());
		if args.features.is_some() {
			diag.warn("--features option has no effect without the --expand-macros flag")
		}
		if args.no_default_features {
			diag.warn("--no-default-features flag has no effect without the --expand-macros flag")
		}
		if args.all_features {
			diag.warn(
				"--all-features flag has no effect without the --expand-macros flag"
			)
		}
		diag.print().unwrap();
	}

	simple_logger::init_with_level(
		args.verbose.then(|| Level::Debug).unwrap_or(Level::Info)
	)
	.expect("Failed to initialize logger");

	let (input_file, template, diagnostics) = read_input(
		args.manifest_path,
		args.bin,
		args.expand_macros,
		args.template,
		args.features,
		args.no_default_features,
		args.all_features
	);
	diagnostics.print().unwrap();
	exit_on_err!(diagnostics);

	let out_is_stdout = args.out.to_str() == Some("-");
	let out = if !out_is_stdout && args.out.is_relative() {
		env::current_dir().unwrap().join(args.out)
	} else {
		args.out
	};

	if args.check {
		info!("Reading {}", out.display());
		match File::open(&out) {
			Ok(mut file) => {
				let check = verify::check_up2date(input_file, &template, &mut file)
					.expect("Failed to check readme");
				check
					.print(out.display().to_string())
					.expect("Unable to write to stderr");
				check.into()
			},
			Err(e) if e.kind() == io::ErrorKind::NotFound => {
				error!("File not found: {}", out.display());
				ExitCode::FAILURE
			},
			Err(e) => {
				error!("Unable to open file {}: {e}", out.display());
				ExitCode::FAILURE
			}
		}
	} else {
		if out_is_stdout {
			info!("Writing README to stdout");
			output::emit(input_file, &template, &mut io::stdout())
				.expect("Unable to write to stdout!");
		} else {
			info!("Writing README to {}", out.display());
			let mut file = File::create(&out).expect("Unable to create output file");
			output::emit(input_file, &template, &mut file)
				.expect("Unable to write output file");
		};
		ExitCode::SUCCESS
	}
}
