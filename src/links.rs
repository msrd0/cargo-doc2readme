use crate::depinfo::DependencyInfo;
use either::Either;
use semver::Version;

pub struct Links {
	pub deps: DependencyInfo
}

impl Links {
	pub fn new(template: &str, rustdoc: &str) -> Self {
		Self {
			deps: DependencyInfo::new(template, rustdoc)
		}
	}

	pub fn build_link(
		&mut self,
		crate_name: &str,
		crate_ver: Option<&Version>,
		search: Option<&str>
	) -> String {
		let lib_name = crate_name.replace('-', "_");
		let link = match search {
			Some(search) => format!(
				"https://docs.rs/{crate_name}/{}/{lib_name}/?search={search}",
				crate_ver
					.map(Either::Left)
					.unwrap_or(Either::Right("latest"))
			),
			None => format!(
				"https://crates.io/crates/{crate_name}{}",
				crate_ver
					.map(|ver| Either::Left(format!("/{ver}")))
					.unwrap_or(Either::Right(""))
			)
		};
		self.deps
			.add_dependency(crate_name.to_owned(), crate_ver.cloned(), lib_name);
		link
	}

	pub fn std_link(&self, search: &str) -> String {
		format!("https://doc.rust-lang.org/stable/std/?search={search}")
	}

	pub fn primitive_link(&self, primitive: &str) -> String {
		format!("https://doc.rust-lang.org/stable/std/primitive.{primitive}.html")
	}
}
