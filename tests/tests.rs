#![warn(rust_2018_idioms)]
#![deny(elided_lifetimes_in_paths)]
#![forbid(unsafe_code)]

use anyhow::bail;
use cargo_doc2readme::{output, read_input};
use lazy_regex::regex_replace_all;
use libtest::{run_tests, Arguments, Outcome, Test};
use pretty_assertions::assert_eq;
use std::{
	fs,
	path::{Path, PathBuf}
};

#[derive(Clone, Copy)]
enum TestType {
	Pass,
	Fail
}

struct TestData {
	manifest_path: PathBuf,
	test_type: TestType
}

fn run_test(data: &TestData) -> anyhow::Result<Outcome> {
	let manifest_path = data.manifest_path.clone();
	let parent = manifest_path.parent().unwrap();
	let template_path = parent.join("README.j2");
	let readme_path = parent.join("README.md");
	let stderr_path = parent.join("stderr.log");

	let (input_file, template, diagnostic) =
		read_input(Some(manifest_path), false, false, template_path);

	let mut stderr = Vec::new();
	diagnostic.print_to(&mut stderr).unwrap();
	let stderr = String::from_utf8(stderr)?;
	let stderr = regex_replace_all!("\x1B\\[[^m]+m", &stderr, |_| "");

	// The program output should always match, no matter if we pass or fail.
	let fail_outcome = if stderr_path.exists() {
		let expected = fs::read_to_string(&stderr_path)?;
		assert_eq!(expected, stderr);
		Ok(Outcome::Passed)
	} else if !stderr.trim().is_empty() {
		fs::write(&stderr_path, stderr.as_bytes())?;
		Ok(Outcome::Ignored)
	} else {
		Ok(Outcome::Failed {
			msg: Some("Missing error message".into())
		})
	};

	let mut actual = Vec::<u8>::new();
	output::emit(input_file, &template, &mut actual)?;

	match (data.test_type, diagnostic.is_fail()) {
		// when passing, also check the readme
		(TestType::Pass, false) => {
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
		(TestType::Fail, true) => fail_outcome,

		// outcome mismatch
		(TestType::Pass, true) => bail!("Expected test to pass, but it failed"),
		(TestType::Fail, false) => bail!("Expected test to fail, but it passed")
	}
}

fn add_tests_from_dir<P>(
	tests: &mut Vec<Test<TestData>>,
	path: P,
	test_type: TestType
) -> anyhow::Result<()>
where
	P: AsRef<Path>
{
	for file in fs::read_dir(path)? {
		let file = file?;
		let path = file.path();
		let ty = file.file_type()?;
		if ty.is_dir() {
			add_tests_from_dir(tests, &path, test_type)?;
		} else if ty.is_file()
			&& path
				.file_name()
				.map(|name| name == "Cargo.toml")
				.unwrap_or(false)
		{
			tests.push(Test {
				name: path.display().to_string(),
				kind: "".into(),
				is_ignored: false,
				is_bench: false,
				data: TestData {
					manifest_path: path,
					test_type
				}
			});
		}
	}
	Ok(())
}

fn main() -> anyhow::Result<()> {
	let args = Arguments::from_args();

	let mut tests = Vec::new();
	add_tests_from_dir(&mut tests, "tests/pass", TestType::Pass)?;
	add_tests_from_dir(&mut tests, "tests/fail", TestType::Fail)?;

	run_tests(&args, tests, |test| match run_test(&test.data) {
		Ok(outcome) => outcome,
		Err(err) => Outcome::Failed {
			msg: Some(format!("{err:?}"))
		}
	})
	.exit();
}
