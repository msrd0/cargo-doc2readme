#![warn(rust_2018_idioms)]
#![deny(elided_lifetimes_in_paths)]
#![forbid(unsafe_code)]

use cargo_doc2readme::{output, read_input};
use libtest::{run_tests, Arguments, Outcome, Test};
use pretty_assertions::assert_eq;
use std::{
	fs::{self, File},
	io::{Read as _, Write},
	path::{Path, PathBuf}
};

struct TestData {
	manifest_path: PathBuf
}

fn run_test(data: &TestData) -> anyhow::Result<Outcome> {
	let manifest_path = data.manifest_path.clone();
	let parent = manifest_path.parent().unwrap();
	let template_path = parent.join("README.j2");
	let readme_path = parent.join("README.md");
	let (input_file, template) = read_input(Some(manifest_path), false, false, template_path);
	let mut actual = Vec::<u8>::new();
	output::emit(input_file, &template, &mut actual)?;

	if readme_path.exists() {
		let actual = String::from_utf8(actual)?;
		let mut expected = String::new();
		File::open(readme_path)?.read_to_string(&mut expected)?;
		assert_eq!(expected, actual);
		Ok(Outcome::Passed)
	} else {
		File::create(readme_path)?.write_all(&actual)?;
		Ok(Outcome::Ignored)
	}
}

fn add_tests_from_dir<P>(tests: &mut Vec<Test<TestData>>, path: P) -> anyhow::Result<()>
where
	P: AsRef<Path>
{
	for file in fs::read_dir(path)? {
		let file = file?;
		let path = file.path();
		let ty = file.file_type()?;
		if ty.is_dir() {
			add_tests_from_dir(tests, &path)?;
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
					manifest_path: path
				}
			});
		}
	}
	Ok(())
}

fn main() -> anyhow::Result<()> {
	let args = Arguments::from_args();

	let mut tests = Vec::new();
	add_tests_from_dir(&mut tests, "tests")?;

	run_tests(&args, tests, |test| match run_test(&test.data) {
		Ok(outcome) => outcome,
		Err(err) => Outcome::Failed {
			msg: Some(format!("{err:?}"))
		}
	})
	.exit();
}
