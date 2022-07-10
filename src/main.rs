#![warn(rust_2018_idioms, rustdoc::broken_intra_doc_links)]
#![deny(elided_lifetimes_in_paths, unsafe_code)]

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

use cargo::{
	core::{
		registry::{LockedPatchDependency, PackageRegistry},
		Dependency, EitherManifest, SourceId, Verbosity
	},
	util::{important_paths::find_root_manifest_for_wd, toml::read_manifest},
	Config as CargoConfig
};
use clap::Parser;
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

	// get the cargo manifest path
	let manifest_path = match args.manifest_path {
		Some(path) if path.is_relative() => env::current_dir().unwrap().join(path),
		Some(path) => path,
		None => find_root_manifest_for_wd(&env::current_dir().unwrap())
			.expect("Unable to find Cargo.toml")
	};

	// initialize cargo
	let cargo_cfg = CargoConfig::default().expect("Failed to initialize cargo");
	cargo_cfg.shell().set_verbosity(
		args.verbose
			.then(|| Verbosity::Verbose)
			.unwrap_or(Verbosity::Normal)
	);

	// parse the cargo manifest
	let src_id = SourceId::for_path(&manifest_path).expect("Failed to obtain source id");
	let manifest =
		read_manifest(&manifest_path, src_id, &cargo_cfg).expect("Failed to read Cargo.toml");
	let manifest = match manifest {
		(EitherManifest::Real(manifest), _) => manifest,
		(EitherManifest::Virtual(_), _) => {
			cargo_cfg
				.shell()
				.error("Virtual manifests (i.e. pure workspace roots) are not supported.")
				.unwrap();
			return ExitCode::FAILURE;
		}
	};

	// find the target whose rustdoc comment we'll use.
	// this uses a library target if exists, otherwise a binary target with the same name as the
	// package, or otherwise the first binary target
	let targets = manifest.targets();
	let target = targets
		.iter()
		.find(|target| target.is_lib())
		.or_else(|| {
			targets
				.iter()
				.find(|target| target.is_bin() && target.name() == manifest.name().as_str())
		})
		.or_else(|| targets.iter().find(|target| target.is_bin()))
		.expect("Failed to find a library or binary target");

	// read crate code
	let file = target
		.src_path()
		.path()
		.expect("Target does not have a source file");
	let code = if args.expand_macros {
		CrateCode::read_expansion(manifest_path.as_path(), target)
			.expect("Failed to read crate code")
	} else {
		CrateCode::read_from_disk(file).expect("Failed to read crate code")
	};

	// initialize the crate registry
	let _guard = cargo_cfg
		.acquire_package_cache_lock()
		.expect("Failed to aquire package cache lock");
	let mut registry =
		PackageRegistry::new(&cargo_cfg).expect("Failed to initialize crate registry");
	for (url, deps) in manifest.patch() {
		let deps: Vec<(&Dependency, Option<LockedPatchDependency>)> =
			deps.iter().map(|dep| (dep, None)).collect();
		registry.patch(url, &deps).expect("Failed to apply patches");
	}
	registry.lock_patches();

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

	// Configure git transport ( makes cargo compatible with HTTP proxies )
	init_git_transports(&cargo_cfg);

	// process the target
	cargo_cfg.shell().status("Reading", file.display()).ok();
	let input_file = input::read_code(&manifest, &mut registry, code).expect("Unable to read file");
	cargo_cfg
		.shell()
		.verbose(|shell| shell.status("Processing", format_args!("{input_file:#?}")))
		.ok();
	if input_file.scope.has_glob_use {
		cargo_cfg.shell().warn("Your code contains glob use statements (e.g. `use std::io::prelude::*;`). Those can lead to incomplete link generation.").ok();
	}
	let out = if args.out.is_relative() {
		env::current_dir().unwrap().join(args.out)
	} else {
		args.out
	};

	let exit_code = if args.check {
		cargo_cfg.shell().status("Reading", out.display()).ok();
		match File::open(&out) {
			Ok(mut file) => {
				let check = verify::check_up2date(input_file, &template, &mut file)
					.expect("Failed to check readme");
				check.print(cargo_cfg.shell());
				check.into()
			},
			Err(e) if e.kind() == io::ErrorKind::NotFound => {
				cargo_cfg
					.shell()
					.error(&format!("File not found: {}", out.display()))
					.ok();
				ExitCode::FAILURE
			},
			Err(e) => panic!("Unable to open file {}: {e}", out.display())
		}
	} else {
		cargo_cfg.shell().status("Writing", out.display()).ok();
		let mut out = File::create(&out).expect("Unable to create output file");
		output::emit(input_file, &template, &mut out).expect("Unable to write output file");
		ExitCode::SUCCESS
	};

	cargo_cfg.release_package_cache_lock();
	exit_code
}

// Copied from cargo crate:
// https://github.com/rust-lang/cargo/blob/e870eac9967b132825116525476d6875c305e4d8/src/bin/cargo/main.rs#L199
fn init_git_transports(config: &CargoConfig) {
	// Only use a custom transport if any HTTP options are specified,
	// such as proxies or custom certificate authorities. The custom
	// transport, however, is not as well battle-tested.

	match cargo::ops::needs_custom_http_transport(config) {
		Ok(true) => {},
		_ => return
	}

	let handle = match cargo::ops::http_handle(config) {
		Ok(handle) => handle,
		Err(..) => return
	};

	// The unsafety of the registration function derives from two aspects:
	//
	// 1. This call must be synchronized with all other registration calls as
	//    well as construction of new transports.
	// 2. The argument is leaked.
	//
	// We're clear on point (1) because this is only called at the start of this
	// binary (we know what the state of the world looks like) and we're mostly
	// clear on point (2) because we'd only free it after everything is done
	// anyway
	#[allow(unsafe_code)]
	unsafe {
		git2_curl::register(handle);
	}
}
