//! `cargo doc2readme` is a cargo subcommand to create a readme file to display on [GitHub] or [crates.io],
//! containing the rustdoc comments from your code.
//!
//! # Usage
//!
//! ```bash
//! cargo install cargo-doc2readme
//! cargo doc2readme --out README.md
//! ```
//!
//! # Features
//!
//!  - parse markdown from your rustdoc comments and embed it into your readme
//!  - support your `[CustomType]` rustdoc links
//!  - default, minimalistic readme template with some useful badges
//!  - custom readme templates
//!
//! # Non-Goals
//!
//!  - verbatim copy of your markdown
//!  - easy readability of the generated markdown source code
//!
//! # Stability Guarantees
//!
//! This project adheres to semantic versioning. All versions will be tested against the latest stable rust version
//! at the time of the release. All non-bugfix changes to the rustdoc input processing and markdown output or the
//! default readme template are considered breaking changes, as well as any non-backwards-compatible changes to the
//! command-line arguments or to these stability guarantees. All other changes, including any changes to the Rust
//! code, or bumping the MSRV, are not considered breaking changes.
//!
//!  [crates.io]: https://crates.io
//!  [GitHub]: https://github.com

use cargo::{
	core::{registry::PackageRegistry, Dependency, EitherManifest, PackageId, SourceId},
	util::{important_paths::find_root_manifest_for_wd, toml::read_manifest},
	Config as CargoConfig
};
use clap::Clap;
use std::{borrow::Cow, env, fs::File, io::Read, path::PathBuf};

mod input;
mod output;

#[derive(Clap)]
enum Subcommand {
	Doc2readme(Args)
}

#[derive(Clap)]
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
	template: PathBuf
}

#[derive(Clap)]
struct CmdLine {
	#[clap(subcommand)]
	cmd: Subcommand
}

fn main() {
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
		None => find_root_manifest_for_wd(&env::current_dir().unwrap()).expect("Unable to find Cargo.toml")
	};

	// parse the cargo manifest
	let cargo_cfg = CargoConfig::default().expect("Failed to initialize cargo");
	let src_id = SourceId::for_path(&manifest_path).expect("Failed to obtain source id");
	let manifest = match read_manifest(&manifest_path, src_id, &cargo_cfg).expect("Failed to read Cargo.toml") {
		(EitherManifest::Real(manifest), _) => manifest,
		(EitherManifest::Virtual(_), _) => panic!("What on earth is a virtual manifest?")
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

	// initialize the crate registry
	let _guard = cargo_cfg
		.acquire_package_cache_lock()
		.expect("Failed to aquire package cache lock");
	let mut registry = PackageRegistry::new(&cargo_cfg).expect("Failed to initialize crate registry");
	for (url, deps) in manifest.patch() {
		let deps: Vec<(&Dependency, Option<(Dependency, PackageId)>)> = deps.iter().map(|dep| (dep, None)).collect();
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

	// process the target
	let file = target.src_path().path().expect("Target does not have a source file");
	cargo_cfg.shell().status("Reading", file.display()).ok();
	let input_file = input::read_file(&manifest, &mut registry, file).expect("Unable to read file");
	cargo_cfg
		.shell()
		.verbose(|shell| shell.status("Processing", format!("{:?}", input_file)))
		.ok();
	if input_file.scope.has_glob_use {
		cargo_cfg.shell().warn("Your code contains glob use statements (e.g. `use std::io::prelude::*;`). Those can lead to incomplete link generation.").ok();
	}
	let out = if args.out.is_relative() {
		env::current_dir().unwrap().join(args.out)
	} else {
		args.out
	};
	cargo_cfg.shell().status("Writing", out.display()).ok();
	let mut out = File::create(out).expect("Unable to create output file");
	output::emit(input_file, &template, &mut out).expect("Unable to write output file");

	cargo_cfg.release_package_cache_lock();
}
