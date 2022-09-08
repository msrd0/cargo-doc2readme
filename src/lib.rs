//! **THIS IS NOT A LIBRARY. NONE OF THE APIS ARE PUBLIC. THEY DON'T
//! ADHERE TO SEMVER. DON'T EVEN USE AT YOUR OWN RISK. DON'T USE IT
//! AT ALL.**

use cargo_metadata::{MetadataCommand, Target};
use log::{debug, info, warn};
use std::{borrow::Cow, env, fs::File, io::Read as _, path::PathBuf};

#[doc(hidden)]
pub mod depinfo;

#[doc(hidden)]
pub mod input;

#[doc(hidden)]
pub mod output;

#[doc(hidden)]
pub mod preproc;

#[doc(hidden)]
pub mod verify;

use input::{CrateCode, InputFile};

#[doc(hidden)]
/// Read input. The manifest path, if present, will be passed to `cargo metadata`. If you set
/// expand_macros to true, the input will be passed to the rust compiler to expand macros. This
/// will only work on a nightly compiler. The template doesn't have to exist, a default will
/// be used if it does not exist.
pub fn read_input(
	manifest_path: Option<PathBuf>,
	prefer_bin: bool,
	expand_macros: bool,
	template: PathBuf
) -> (InputFile, Cow<'static, str>) {
	// get the cargo manifest path
	let manifest_path = match manifest_path {
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
	let target = if prefer_bin {
		pkg.targets
			.iter()
			.find(is_bin)
			.or_else(|| pkg.targets.iter().find(is_lib))
	} else {
		pkg.targets
			.iter()
			.find(is_lib)
			.or_else(|| pkg.targets.iter().find(is_bin))
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
	let code = if expand_macros {
		CrateCode::read_expansion(manifest_path.as_ref(), target)
			.expect("Failed to read crate code")
	} else {
		CrateCode::read_from_disk(file).expect("Failed to read crate code")
	};

	// resolve the template
	let template: Cow<'static, str> = if template.exists() {
		let mut buf = String::new();
		File::open(template)
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

	(input_file, template)
}
