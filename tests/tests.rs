#![warn(rust_2018_idioms)]
#![deny(elided_lifetimes_in_paths)]
#![forbid(unsafe_code)]

use anyhow::bail;
use cargo_doc2readme::{output, read_input, verify};
use lazy_regex::regex_replace_all;
use libtest::{run_tests, Arguments, Outcome, Test};
use pretty_assertions::assert_eq;
use std::{
	fs::{self, File},
	panic::catch_unwind,
	path::{Path, PathBuf}
};

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
	test_type: TestType
}

fn sanitize_stderr(stderr: Vec<u8>) -> anyhow::Result<String> {
	let stderr = String::from_utf8(stderr)?;
	Ok(regex_replace_all!("\x1B\\[[^m]+m", &stderr, |_| "").into_owned())
}

fn run_test(data: &TestData) -> anyhow::Result<Outcome> {
	let manifest_path = data.manifest_path.clone();
	let parent = manifest_path.parent().unwrap();
	let template_path = parent.join("README.j2");
	let readme_path = parent.join("README.md");
	let stderr_path = parent.join("stderr.log");

	let (input_file, template, diagnostic) = read_input(
		Some(manifest_path),
		false,
		false,
		template_path,
		None,
		false,
		false
	);

	let mut stderr = Vec::new();
	diagnostic.print_to(&mut stderr).unwrap();
	let stderr = sanitize_stderr(stderr)?;

	// The program output should always match, no matter if we pass or fail.
	let fail_outcome = match data.test_type {
		TestType::ReadmePass | TestType::ReadmeFail => Some(if stderr_path.exists() {
			let expected = fs::read_to_string(&stderr_path)?;
			assert_eq!(expected, stderr);
			Outcome::Passed
		} else if !stderr.trim().is_empty() {
			fs::write(&stderr_path, stderr.as_bytes())?;
			Outcome::Ignored
		} else {
			Outcome::Failed {
				msg: Some("Missing error message".into())
			}
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
				Ok(Outcome::Passed)
			} else {
				fs::write(&readme_path, &actual)?;
				Ok(Outcome::Ignored)
			}
		},

		// when failing, no readme check is required
		(TestType::ReadmeFail, true) => Ok(fail_outcome.unwrap()),

		// expect check to pass
		(TestType::CheckPass, false) => {
			if readme_path.exists() {
				let mut file = File::open(readme_path)?;
				let check = verify::check_up2date(input_file, &template, &mut file)?;
				if check.is_ok() {
					Ok(Outcome::Passed)
				} else {
					Ok(Outcome::Failed {
						msg: Some("Expected check to pass, but it failed".into())
					})
				}
			} else {
				Ok(Outcome::Ignored)
			}
		},

		// expect check to fail
		(TestType::CheckFail, true) => Ok(Outcome::Passed),
		(TestType::CheckFail, false) => {
			if readme_path.exists() {
				let mut file = File::open(readme_path)?;
				let check = verify::check_up2date(input_file, &template, &mut file)?;
				if check.is_ok() {
					Ok(Outcome::Failed {
						msg: Some("Expected check to fail, but it passed".into())
					})
				} else {
					let mut stderr = Vec::new();
					check.print_to("README.md", &mut stderr).unwrap();
					let stderr = sanitize_stderr(stderr)?;

					Ok(if stderr_path.exists() {
						let expected = fs::read_to_string(&stderr_path)?;
						assert_eq!(expected, stderr);
						Outcome::Passed
					} else if !stderr.trim().is_empty() {
						fs::write(&stderr_path, stderr.as_bytes())?;
						Outcome::Ignored
					} else {
						Outcome::Failed {
							msg: Some("Missing error message".into())
						}
					})
				}
			} else {
				Ok(Outcome::Failed {
					msg: Some("Missing README.md file to check against".into())
				})
			}
		},

		// outcome mismatch
		(TestType::ReadmePass, true) => bail!("Expected test to pass, but it failed"),
		(TestType::CheckPass, true) => bail!("Expected check to pass, but it failed"),
		(TestType::ReadmeFail, false) => bail!("Expected test to fail, but it passed")
	}
}

fn add_tests_from_dir<P, I>(
	tests: &mut Vec<Test<TestData>>,
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
				tests.push(Test {
					name: format!("{} ({test_type:?})", path.display()),
					kind: "".into(),
					is_ignored: false,
					is_bench: false,
					data: TestData {
						manifest_path: path.clone(),
						test_type
					}
				});
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

	run_tests(&args, tests, |test| {
		match catch_unwind(|| run_test(&test.data)) {
			Ok(result) => match result {
				Ok(outcome) => outcome,
				Err(err) => Outcome::Failed {
					msg: Some(format!("{err:?}"))
				}
			},
			Err(_) => Outcome::Failed { msg: None }
		}
	})
	.exit();
}
