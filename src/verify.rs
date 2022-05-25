use crate::{depinfo::DependencyInfo, input::InputFile, output};
use cargo::core::Shell;
use memchr::{memchr2, memmem};
use std::{cell::RefMut, io, process::ExitCode};

pub enum Check {
	/// Everything is up to date.
	UpToDate,

	/// The dependency info is malformed.
	InvalidDepInfo(anyhow::Error),

	/// The input (template or rustdoc) have changed.
	InputChanged,

	/// One or more dependencies use an incompatible version.
	IncompatibleVersion,

	/// Input and output are different (no dep info was included).
	OutputChanged
}

impl Check {
	pub fn print(&self, mut shell: RefMut<'_, Shell>) {
		match self {
			Check::UpToDate => shell.note("Readme is up to date"),
			Check::InvalidDepInfo(e) => {
				shell.error(format!("Readme has invalid dependency info: {e}"))
			},
			Check::InputChanged => shell.error("Input has changed"),
			Check::IncompatibleVersion => {
				shell.error("Readme links to incompatible dependency version")
			},
			Check::OutputChanged => shell.error("Readme has changed")
		}
		.ok();
	}
}

impl From<Check> for ExitCode {
	fn from(check: Check) -> Self {
		match check {
			Check::UpToDate => Self::SUCCESS,
			_ => Self::FAILURE
		}
	}
}

pub fn check_up2date(
	input: InputFile,
	template: &str,
	check_file: &mut dyn io::Read
) -> anyhow::Result<Check> {
	let mut check_buf = Vec::new();
	check_file.read_to_end(&mut check_buf)?;

	let search_key = b" [__cargo_doc2readme_dependencies_info]: ";
	if let Some(search_idx) = memmem::find(&check_buf, search_key) {
		let sub = &check_buf[search_idx + search_key.len()..];
		let end_idx = memchr2(b' ', b'\n', sub).unwrap_or(sub.len());
		let depinfo_str = String::from_utf8(sub[..end_idx].to_vec()).unwrap();
		let depinfo = match DependencyInfo::decode(depinfo_str) {
			Ok(depinfo) => depinfo,
			Err(e) => {
				return Ok(Check::InvalidDepInfo(e));
			}
		};

		// ensure the input is up to date
		if !depinfo.check_input(template, &input.rustdoc) {
			return Ok(Check::InputChanged);
		}

		// ensure that the dependencies that were used in the readme still meet the current required
		// versions. dependencies that are missing in the readme don't matter.
		for (lib_name, (crate_name, version)) in &input.dependencies {
			if !depinfo.check_dependency(crate_name, Some(version), lib_name, true) {
				return Ok(Check::IncompatibleVersion);
			}
		}

		// looks like everything is up to date
		return Ok(Check::UpToDate);
	}

	// if no dependency info was available, do a bytewise comparison
	let mut output_buf = Vec::new();
	output::emit(input, template, &mut output_buf)?;
	Ok(if output_buf == check_buf {
		Check::UpToDate
	} else {
		Check::OutputChanged
	})
}
