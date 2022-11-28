//! **THIS IS NOT A LIBRARY. NONE OF THE APIS ARE PUBLIC. THEY DON'T
//! ADHERE TO SEMVER. DON'T EVEN USE AT YOUR OWN RISK. DON'T USE IT
//! AT ALL.**

use cargo_metadata::{CargoOpt, MetadataCommand, Target};
use log::{debug, info};
use std::{borrow::Cow, collections::HashMap, env, fmt::Display, fs, path::PathBuf};

#[doc(hidden)]
pub mod depinfo;
#[doc(hidden)]
pub mod diagnostic;
#[doc(hidden)]
pub mod input;
#[doc(hidden)]
pub mod links;
#[doc(hidden)]
pub mod output;
#[doc(hidden)]
pub mod preproc;
#[doc(hidden)]
pub mod verify;

use crate::input::Scope;
use diagnostic::Diagnostic;
use input::{CrateCode, InputFile, TargetType};

#[doc(hidden)]
#[allow(clippy::too_many_arguments)] // TODO
/// Read input. The manifest path, if present, will be passed to `cargo metadata`. If you set
/// expand_macros to true, the input will be passed to the rust compiler to expand macros. This
/// will only work on a nightly compiler. The template doesn't have to exist, a default will
/// be used if it does not exist.
pub fn read_input(
	manifest_path: Option<PathBuf>,
	package: Option<String>,
	prefer_bin: bool,
	expand_macros: bool,
	template: PathBuf,
	features: Option<String>,
	no_default_features: bool,
	all_features: bool
) -> (InputFile, Cow<'static, str>, Diagnostic) {
	/// Create a fake input when reading the input failed before we had any code.
	fn fail<T: Display>(msg: T) -> (InputFile, Cow<'static, str>, Diagnostic) {
		let input = InputFile {
			crate_name: "N/A".into(),
			target_type: TargetType::Lib,
			repository: None,
			license: None,
			rust_version: None,
			rustdoc: String::new(),
			dependencies: HashMap::new(),
			scope: Scope::empty()
		};
		let template = "".into();
		let mut diagnostic = Diagnostic::new("<none>".into(), String::new());
		diagnostic.error(msg);
		(input, template, diagnostic)
	}

	trait Fail {
		type Ok;

		fn fail(self, msg: &'static str) -> Result<Self::Ok, Cow<'static, str>>;
	}

	impl<T> Fail for Option<T> {
		type Ok = T;

		fn fail(self, msg: &'static str) -> Result<Self::Ok, Cow<'static, str>> {
			self.ok_or(Cow::Borrowed(msg))
		}
	}

	impl<T, E: Display> Fail for Result<T, E> {
		type Ok = T;

		fn fail(self, msg: &'static str) -> Result<T, Cow<'static, str>> {
			self.map_err(|err| format!("{msg}: {err}").into())
		}
	}

	macro_rules! unwrap {
		($expr:expr) => {
			match $expr {
				Ok(ok) => ok,
				Err(err) => return fail(err)
			}
		};

		($expr:expr, $msg:literal) => {
			match Fail::fail($expr, $msg) {
				Ok(ok) => ok,
				Err(err) => return fail(err)
			}
		};
	}

	// get the cargo manifest path
	let manifest_path = match manifest_path {
		Some(path) if path.is_relative() => Some(env::current_dir().unwrap().join(path)),
		Some(path) => Some(path),
		None => None
	};

	// parse the cargo metadata
	let mut cmd = MetadataCommand::new();
	cmd.features(CargoOpt::AllFeatures);
	if let Some(path) = &manifest_path {
		cmd.manifest_path(path);
	}
	let metadata = unwrap!(cmd.exec(), "Failed to get cargo metadata");
	let pkg = match package {
		Some(package) => unwrap!(
			metadata.packages.iter().find(|pkg| pkg.name == package),
			"Cannot find requested package"
		),
		None => unwrap!(
			metadata.root_package(),
			"Missing package. Please make sure there is a package here, workspace roots don't contain any documentation."
		)
	};

	// find the target whose rustdoc comment we'll use.
	// this uses a library target if exists, otherwise a binary target with the same name as the
	// package, or otherwise the first binary target
	let is_lib = |target: &&Target| target.is_lib();
	let is_default_bin =
		|target: &&Target| target.is_bin() && target.name == pkg.name.as_str();
	let target_and_type = if prefer_bin {
		pkg.targets
			.iter()
			.find(is_default_bin)
			.map(|target| (target, TargetType::Bin))
			.or_else(|| {
				pkg.targets
					.iter()
					.find(is_lib)
					.map(|target| (target, TargetType::Lib))
			})
	} else {
		pkg.targets
			.iter()
			.find(is_lib)
			.map(|target| (target, TargetType::Lib))
			.or_else(|| {
				pkg.targets
					.iter()
					.find(is_default_bin)
					.map(|target| (target, TargetType::Bin))
			})
	};
	let (target, target_type) = unwrap!(
		target_and_type.or_else(|| {
			pkg.targets
				.iter()
				.find(|target| target.is_bin())
				.map(|target| (target, TargetType::Bin))
		}),
		"Failed to find a library or binary target"
	);

	// resolve the template
	let template: Cow<'static, str> = if template.exists() {
		unwrap!(fs::read_to_string(template), "Failed to read template").into()
	} else {
		include_str!("README.j2").into()
	};

	// read crate code
	let file = target.src_path.as_std_path();
	let filename = file
		.file_name()
		.expect("File has no filename")
		.to_string_lossy()
		.into_owned();
	let code = if expand_macros {
		unwrap!(
			CrateCode::read_expansion(
				manifest_path.as_ref(),
				target,
				features,
				no_default_features,
				all_features
			),
			"Failed to read crate code"
		)
	} else {
		unwrap!(CrateCode::read_from_disk(file), "Failed to read crate code")
	};
	let mut diagnostics = Diagnostic::new(filename, code.0.clone());

	// process the target
	info!("Reading {}", file.display());
	let input_file =
		input::read_code(&metadata, pkg, code, target_type, &mut diagnostics);
	debug!("Processing {input_file:#?}");

	(input_file, template, diagnostics)
}
