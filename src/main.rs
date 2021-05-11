//! `cargo doc2readme` is a cargo subcommand to create a readme file to display on [GitHub] or [crates.io],
//! containing the rustdoc comments from your code.
//!
//!  [crates.io]: https://crates.io
//!  [GitHub]: https://github.com

use cargo::{
	core::{registry::PackageRegistry, Dependency, EitherManifest, PackageId, SourceId},
	util::{important_paths::find_root_manifest_for_wd, toml::read_manifest},
	Config as CargoConfig
};
use clap::Clap;
use std::{env, fs::File, path::PathBuf};

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
	#[clap(short, long)]
	out: PathBuf
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

	// process the target
	let file = target.src_path().path().expect("Target does not have a source file");
	cargo_cfg.shell().status("Reading", file.display()).ok();
	let input_file = input::read_file(&manifest, &mut registry, file).expect("Unable to read file");
	cargo_cfg
		.shell()
		.verbose(|shell| shell.status("Processing", format!("{:?}", input_file)))
		.ok();
	let out = if args.out.is_relative() {
		env::current_dir().unwrap().join(args.out)
	} else {
		args.out
	};
	cargo_cfg.shell().status("Writing", out.display()).ok();
	let mut out = File::create(out).expect("Unable to create output file");
	output::emit(input_file, &mut out).expect("Unable to write output file");

	cargo_cfg.release_package_cache_lock();
}
