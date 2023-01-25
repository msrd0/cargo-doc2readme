#![warn(rust_2018_idioms)]
#![deny(elided_lifetimes_in_paths)]
#![forbid(unsafe_code)]

use cargo_doc2readme::{
	diagnostic::Diagnostic, input::InputFile, output, read_input, verify
};
use lazy_regex::regex_replace_all;
use libtest::{Arguments, Failed, Trial};
use pretty_assertions::assert_eq;
use serde::Deserialize;
use std::{
	borrow::Cow,
	fs::{self, File},
	io,
	panic::catch_unwind,
	path::{Path, PathBuf}
};

/// This can be loaded from a `test.toml` in the test directory and alter the behaviour
/// that is being tested.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestConfig {
	/// Test requires nightly Rust.
	#[serde(default)]
	nightly: bool,

	/// Test requires that diagnostics match the content of `stderr.log`.
	#[serde(default)]
	stderr: bool,

	/// Test as if `--expand-macros` was passed.
	#[serde(default)]
	expand_macros: bool,

	/// Test with these features enabled. Ignored unless combined with `--expand-macros`.
	features: Option<String>,

	/// Test with all features enabled. Ignored unless combined with `--expand-macros`.
	#[serde(default)]
	all_features: bool,

	/// Test without default feature being enabled. Ignored unless combined with
	/// `--expand-macros`.
	#[serde(default)]
	no_default_features: bool
}

#[derive(Clone, Copy, Debug)]
enum TestType {
	/// Test that the readme generation passes, and the output matches exactly the test
	/// case.
	ReadmePass,

	/// Test that the readme check reports that everything is up to date.
	CheckPass,

	/// Test that the readme generation fails, and that correct error message was printed
	/// to stderr.
	ReadmeFail,

	/// Test that the readme check reports that the readme needs updating.
	CheckFail
}

struct TestData {
	manifest_path: PathBuf,
	test_type: TestType,
	config: TestConfig
}

fn sanitize_stderr(stderr: Vec<u8>) -> anyhow::Result<String> {
	let stderr = String::from_utf8(stderr)?;
	Ok(regex_replace_all!("\x1B\\[[^m]+m", &stderr, |_| "").into_owned())
}

struct TestRun<'a> {
	data: &'a TestData,

	manifest_path: PathBuf,
	template_path: PathBuf,
	readme_path: PathBuf,
	stderr_path: PathBuf,

	input_file: InputFile,
	template: Cow<'static, str>,
	diagnostic: Diagnostic
}

impl<'a> TestRun<'a> {
	/// Read the input for this test.
	fn init(data: &'a TestData) -> Self {
		let manifest_path = data.manifest_path.clone();
		let parent = manifest_path.parent().unwrap();
		let template_path = parent.join("README.j2");
		let readme_path = parent.join("README.md");
		let stderr_path = parent.join("stderr.log");

		let (input_file, template, diagnostic) = read_input(
			Some(&manifest_path),
			None,
			false,
			data.config.expand_macros,
			&template_path,
			data.config.features.clone(),
			data.config.no_default_features,
			data.config.all_features
		);

		Self {
			data,
			manifest_path,
			template_path,
			readme_path,
			stderr_path,
			input_file,
			template,
			diagnostic
		}
	}

	fn check_stderr(&self) -> Result<(), Failed> {
		let mut stderr = Vec::new();
		self.diagnostic.print_to(&mut stderr).unwrap();
		let stderr = sanitize_stderr(stderr)?;

		if self.stderr_path.exists() {
			let expected = fs::read_to_string(&self.stderr_path)?;
			assert_eq!(expected, stderr);
			Ok(())
		} else if !stderr.trim().is_empty() {
			fs::write(&self.stderr_path, stderr.as_bytes())?;
			Err("WIP".into())
		} else {
			Err("Missing diagnostics".into())
		}
	}

	/// Run this to check if the generated readme (and diagnostics) match the expected
	/// results.
	fn check_readme_pass(self) -> Result<(), Failed> {
		if self.diagnostic.is_fail() {
			return Err("Expected test to pass, but it failed".into());
		}

		if self.data.config.stderr {
			self.check_stderr()?;
		}

		let mut actual = Vec::<u8>::new();
		output::emit(self.input_file, &self.template, &mut actual)?;

		if self.readme_path.exists() {
			let actual = String::from_utf8(actual)?;
			let expected = fs::read_to_string(&self.readme_path)?;
			assert_eq!(expected, actual);
		} else {
			fs::write(&self.readme_path, &actual)?;
			return Err("WIP".into());
		}

		Ok(())
	}

	fn check_readme_fail(self) -> Result<(), Failed> {
		if !self.diagnostic.is_fail() {
			return Err("Expected test to fail, but it passed".into());
		}

		if self.data.config.stderr {
			self.check_stderr()?;
		} else {
			println!(
				"[WARN] {} has no diagnostic check",
				self.readme_path.display()
			);
		}
		Ok(())
	}
}

fn run_test(data: &TestData) -> Result<(), Failed> {
	let manifest_path = data.manifest_path.clone();
	let parent = manifest_path.parent().unwrap();
	let template_path = parent.join("README.j2");
	let readme_path = parent.join("README.md");
	let stderr_path = parent.join("stderr.log");

	let (input_file, template, diagnostic) = read_input(
		Some(manifest_path),
		None,
		false,
		data.config.expand_macros,
		template_path,
		data.config.features.clone(),
		data.config.no_default_features,
		data.config.all_features
	);

	let stderr = if data.config.stderr {
		let mut stderr = Vec::new();
		diagnostic.print_to(&mut stderr).unwrap();
		sanitize_stderr(stderr)?
	} else {
		// leaving stderr empty means we won't check it unless stderr.log exists
		// this could be improved but works for now
		String::new()
	};

	// The program output should always match, no matter if we pass or fail.
	let fail_outcome = match data.test_type {
		TestType::ReadmePass | TestType::ReadmeFail => Some(if stderr_path.exists() {
			let expected = fs::read_to_string(&stderr_path)?;
			assert_eq!(expected, stderr);
			Ok(())
		} else if !stderr.trim().is_empty() {
			fs::write(&stderr_path, stderr.as_bytes())?;
			Err("WIP".into())
		} else {
			Err("Missing error message".into())
		}),
		TestType::CheckPass | TestType::CheckFail => None
	};

	match (data.test_type, diagnostic.is_fail()) {
		// when passing, also check the readme
		(TestType::ReadmePass, false) => {
			let mut actual = Vec::<u8>::new();
			output::emit(input_file, &template, &mut actual)?;

			if readme_path.exists() {
				let actual = String::from_utf8(actual)?;
				let expected = fs::read_to_string(&readme_path)?;
				assert_eq!(expected, actual);
				Ok(())
			} else {
				fs::write(&readme_path, &actual)?;
				Err("WIP".into())
			}
		},

		// when failing, no readme check is required
		(TestType::ReadmeFail, true) => fail_outcome.unwrap(),

		// expect check to pass
		(TestType::CheckPass, false) => {
			if readme_path.exists() {
				let mut file = File::open(readme_path)?;
				let check = verify::check_up2date(input_file, &template, &mut file)?;
				if check.is_ok() {
					Ok(())
				} else {
					Err("Expected check to pass, but it failed".into())
				}
			} else {
				Err("WIP".into())
			}
		},

		// expect check to fail
		(TestType::CheckFail, true) => Ok(()),
		(TestType::CheckFail, false) => {
			if readme_path.exists() {
				let mut file = File::open(readme_path)?;
				let check = verify::check_up2date(input_file, &template, &mut file)?;
				if check.is_ok() {
					Err("Expected check to fail, but it passed".into())
				} else {
					let mut stderr = Vec::new();
					check.print_to("README.md", &mut stderr).unwrap();
					let stderr = sanitize_stderr(stderr)?;

					if stderr_path.exists() {
						let expected = fs::read_to_string(&stderr_path)?;
						assert_eq!(expected, stderr);
						Ok(())
					} else if !stderr.trim().is_empty() {
						fs::write(&stderr_path, stderr.as_bytes())?;
						Err("WIP".into())
					} else {
						Err("Missing error message".into())
					}
				}
			} else {
				Err("Missing README.md file to check against".into())
			}
		},

		// outcome mismatch
		(TestType::ReadmePass, true) => {
			Err("Expected test to pass, but it failed".into())
		},
		(TestType::CheckPass, true) => {
			Err("Expected check to pass, but it failed".into())
		},
		(TestType::ReadmeFail, false) => {
			Err("Expected test to fail, but it passed".into())
		},
	}
}

fn add_tests_from_dir<P, I>(
	tests: &mut Vec<Trial>,
	path: P,
	test_types: I,
	recursive: bool
) -> anyhow::Result<()>
where
	P: AsRef<Path>,
	I: IntoIterator<Item = TestType> + Copy
{
	for file in fs::read_dir(path)? {
		let file = file?;
		let path = file.path();
		let ty = file.file_type()?;
		if ty.is_dir() && recursive {
			add_tests_from_dir(tests, &path, test_types, false)?;
		} else if ty.is_file()
			&& path
				.file_name()
				.map(|name| name == "Cargo.toml")
				.unwrap_or(false)
		{
			for test_type in test_types {
				// load test config
				let test_config_path = path.parent().unwrap().join("Config.toml");
				let test_config = fs::read_to_string(&test_config_path);
				let test_config = match test_config {
					Err(err) => {
						if err.kind() == io::ErrorKind::NotFound {
							None
						} else {
							panic!("{}: {}", test_config_path.display(), err);
						}
					},
					Ok(value) => Some(value)
				};
				let test_config = if let Some(test_config) = test_config {
					toml::from_str(&test_config).unwrap()
				} else {
					TestConfig::default()
				};

				if test_config.nightly && !rustversion::cfg!(nightly) {
					continue;
				}

				let name = format!("{} ({test_type:?})", path.display());
				let manifest_path = path.clone();
				tests.push(Trial::test(name, move || {
					let data = TestData {
						manifest_path,
						test_type,
						config: test_config
					};

					match catch_unwind(|| run_test(&data)) {
						Ok(result) => result,
						Err(_) => Err(Failed::without_message())
					}
				}));
			}
		}
	}
	Ok(())
}

fn main() -> anyhow::Result<()> {
	let args = Arguments::from_args();

	use TestType::*;
	let mut tests = Vec::new();
	add_tests_from_dir(&mut tests, "tests/pass", [ReadmePass, CheckPass], true)?;
	add_tests_from_dir(&mut tests, "tests/fail", [ReadmeFail], true)?;
	add_tests_from_dir(&mut tests, "tests/check", [CheckFail], true)?;

	libtest::run(&args, tests).exit()
}
