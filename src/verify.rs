use crate::{depinfo::DependencyInfo, diagnostic::Diagnostic, input::InputFile, output};
use log::debug;
use memchr::{memchr2, memmem};
use std::{io, process::ExitCode};

pub enum Check {
	/// Everything is up to date.
	UpToDate,

	/// The dependency info is malformed.
	InvalidDepInfo(anyhow::Error),

	/// The input (template or rustdoc) have changed.
	InputChanged,

	/// One or more dependencies use an incompatible version.
	IncompatibleVersion(String),

	/// The readme used an outdated "markdown version".
	OutdatedMarkdown,

	/// Input and output are different (no dep info was included).
	OutputChanged
}

impl Check {
	pub fn print<T: Into<String>>(&self, filename: T) -> io::Result<()> {
		self.print_to(filename, io::stderr())
	}

	pub fn print_to<T, W>(&self, filename: T, out: W) -> Result<(), io::Error>
	where
		T: Into<String>,
		W: io::Write
	{
		let mut diag = Diagnostic::new(filename.into(), String::new());
		match self {
			Check::UpToDate => {
				diag.info("Readme is up to date");
			},
			Check::InvalidDepInfo(e) => {
				diag.warn(format_args!("Readme has invalid dependency info: {e}"));
			},
			Check::InputChanged => diag.error("Input has changed"),
			Check::IncompatibleVersion(name) => {
				diag.error(format_args!(
					"Readme links to incompatible version of dependency `{name}`"
				));
			},
			Check::OutdatedMarkdown => {
				diag.error(
					"The readme was created with an outdated version of this tool"
				);
			},
			Check::OutputChanged => {
				diag.error("Readme has changed");
			}
		}
		diag.print_to(out)
	}

	pub fn is_ok(&self) -> bool {
		matches!(self, Self::UpToDate)
	}
}

impl From<Check> for ExitCode {
	fn from(check: Check) -> Self {
		match check.is_ok() {
			true => Self::SUCCESS,
			false => Self::FAILURE
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
		let sub = &check_buf[search_idx + search_key.len() ..];
		let end_idx = memchr2(b' ', b'\n', sub).unwrap_or(sub.len());
		let depinfo_str = String::from_utf8(sub[.. end_idx].to_vec()).unwrap();
		let depinfo = match DependencyInfo::decode(depinfo_str) {
			Ok(depinfo) => depinfo,
			Err(e) => {
				return Ok(Check::InvalidDepInfo(e));
			}
		};

		// ensure markdown version matches
		if depinfo.check_outdated() {
			return Ok(Check::OutdatedMarkdown);
		}

		// ensure the input is up to date
		if !depinfo.check_input(template, &input.rustdoc) {
			return Ok(Check::InputChanged);
		}

		// ensure that the dependencies that were used in the readme still meet the current required
		// versions. dependencies that are missing in the readme don't matter.
		for (lib_name, dep) in &input.dependencies {
			debug!("Checking {} = \"{}\"", dep.crate_name, dep.req);
			if !depinfo.check_dependency(&dep.crate_name, Some(&dep.req), lib_name, true)
			{
				return Ok(Check::IncompatibleVersion(dep.crate_name.clone()));
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
