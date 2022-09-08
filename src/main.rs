#![warn(rust_2018_idioms, rustdoc::broken_intra_doc_links)]
#![deny(elided_lifetimes_in_paths)]
#![forbid(unsafe_code)]

//! `cargo doc2readme` is a cargo subcommand to create a readme file to display on
//! [GitHub] or [crates.io],
//! containing the rustdoc comments from your code.
//!
//! # Usage
//!
//! ```bash
//! cargo install cargo-doc2readme
//! cargo doc2readme --out README.md
//! ```
//!
//! If you want to run this using GitHub Actions, you can use the pre-built docker image:
//!
//! ```yaml
//! readme:
//!   runs-on: ubuntu-latest
//!   steps:
//!     - uses: actions/checkout@v2
//!     - uses: docker://ghcr.io/msrd0/cargo-doc2readme
//!       with:
//!         entrypoint: cargo
//!         args: doc2readme --check
//! ```
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

use clap::Parser;
use log::{debug, error, info, warn, Level};
use std::{
	borrow::Cow,
	env,
	fs::File,
	io::{self, Read},
	path::PathBuf,
	process::ExitCode
};

mod depinfo;
mod input;
mod output;
mod verify;

use cargo_metadata::{MetadataCommand, Target};
use input::CrateCode;

#[derive(Parser)]
enum Subcommand {
	Doc2readme(Args)
}

#[derive(Parser)]
struct Args {
	/// Path to Cargo.toml.
	#[clap(long)]
	manifest_path: Option<PathBuf>,

	/// Output File.
	#[clap(short, long, default_value = "README.md")]
	out: PathBuf,

	/// Template File. This is processed by Tera. Look at the source code for cargo-doc2readme for
	/// an example.
	#[clap(short, long, default_value = "README.j2")]
	template: PathBuf,

	/// Use nightly rustc to expand macros prior to reading the source. This is necessary if you
	/// use function-like macros in doc attributes, as introduced in Rust 1.54.
	#[clap(long)]
	expand_macros: bool,

	/// Prefer binary targets over library targets for rustdoc source.
	#[clap(long, conflicts_with = "lib")]
	bin: bool,

	/// Prefer library targets over binary targets for rustdoc source. This is the default.
	#[clap(long, conflicts_with = "bin")]
	lib: bool,

	/// Verify that the output file is (reasonably) up to date, and fail
	/// if it needs updating. The output file will not be changed.
	#[clap(long)]
	check: bool,

	/// Enable verbose output.
	#[clap(short, long)]
	verbose: bool
}

#[derive(Parser)]
struct CmdLine {
	#[clap(subcommand)]
	cmd: Subcommand
}

fn main() -> ExitCode {
	let args = match env::args().nth(1) {
		Some(subcmd) if subcmd == "doc2readme" => match CmdLine::parse().cmd {
			Subcommand::Doc2readme(args) => args
		},
		_ => Args::parse()
	};

	simple_logger::init_with_level(args.verbose.then(|| Level::Debug).unwrap_or(Level::Info))
		.expect("Failed to initialize logger");

	// get the cargo manifest path
	let manifest_path = match args.manifest_path {
		Some(path) if path.is_relative() => Some(env::current_dir().unwrap().join(path)),
		Some(path) => Some(path),
		None => None
	};

	// parse the cargo metadata
	let mut cmd = MetadataCommand::new();
	if let Some(path) = &manifest_path {
		cmd.manifest_path(path);
	}
	let metadata = cmd.exec().expect("Failed to get cargo metadata");
	let pkg = metadata
		.root_package()
		.expect("Missing root package; did you call this command on a workspace root?");

	// find the target whose rustdoc comment we'll use.
	// this uses a library target if exists, otherwise a binary target with the same name as the
	// package, or otherwise the first binary target
	let is_lib = |target: &&Target| target.kind.iter().any(|kind| kind == "lib");
	let is_bin = |target: &&Target| {
		target.kind.iter().any(|kind| kind == "bin") && target.name == pkg.name.as_str()
	};
	let target = if args.bin {
		pkg.targets
			.iter()
			.find(is_lib)
			.or_else(|| pkg.targets.iter().find(is_bin))
	} else {
		pkg.targets
			.iter()
			.find(is_bin)
			.or_else(|| pkg.targets.iter().find(is_lib))
	};
	let target = target
		.or_else(|| {
			pkg.targets
				.iter()
				.find(|target| target.kind.iter().any(|kind| kind == "bin"))
		})
		.expect("Failed to find a library or binary target");

	// read crate code
	let file = target.src_path.as_std_path();
	let code = if args.expand_macros {
		CrateCode::read_expansion(manifest_path.as_ref(), target)
			.expect("Failed to read crate code")
	} else {
		CrateCode::read_from_disk(file).expect("Failed to read crate code")
	};

	// resolve the template
	let template: Cow<'static, str> = if args.template.exists() {
		let mut buf = String::new();
		File::open(args.template)
			.expect("Failed to open template")
			.read_to_string(&mut buf)
			.expect("Failed to read template");
		buf.into()
	} else {
		include_str!("README.j2").into()
	};

	// process the target
	info!("Reading {}", file.display());
	let input_file = input::read_code(&metadata, pkg, code).expect("Unable to read file");
	debug!("Processing {input_file:#?}");
	if input_file.scope.has_glob_use {
		warn!("Your code contains glob use statements (e.g. `use std::io::prelude::*;`). Those can lead to incomplete link generation.");
	}
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
				check.print();
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
			output::emit(input_file, &template, &mut file).expect("Unable to write output file");
		};
		ExitCode::SUCCESS
	}
}
