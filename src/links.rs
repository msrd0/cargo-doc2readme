use crate::{
	depinfo::DependencyInfo,
	input::{Dependency, InputFile}
};
use either::Either;
use itertools::Itertools as _;
use semver::Version;
use syn::Path;

const RUST_PRIMITIVES: &[&str] = &[
	// https://doc.rust-lang.org/stable/std/primitive/index.html#reexports
	"bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "str",
	"u128", "u16", "u32", "u64", "u8", "usize"
];

pub struct Links {
	pub deps: DependencyInfo
}

impl Links {
	pub fn new(template: &str, rustdoc: &str) -> Self {
		Self {
			deps: DependencyInfo::new(template, rustdoc)
		}
	}

	pub fn build_link(&mut self, path: &Path, input: &InputFile) -> String {
		let first = path
			.segments
			.first()
			.map(|segment| segment.ident.to_string())
			.unwrap_or_default();
		// remove all arguments so that `Vec<String>` points to Vec
		let search = path
			.segments
			.iter()
			.filter_map(|segment| match segment.ident.to_string() {
				ident if ident == "crate" => None,
				ident => Some(ident)
			})
			.join("::");

		// TODO more sophisticated link generation
		if first == "std" || first == "alloc" || first == "core" {
			self.std_link(search.as_str())
		} else if first == "crate" {
			let (crate_name, crate_ver) = input
				.dependencies
				.get(&input.crate_name)
				.map(Dependency::as_tuple)
				.unwrap_or((&input.crate_name, None));
			self.build_link_impl(crate_name, crate_ver, Some(&search))
		} else if path.segments.len() > 1 {
			let (crate_name, crate_ver) = input
				.dependencies
				.get(&first)
				.map(Dependency::as_tuple)
				.unwrap_or((&first, None));
			self.build_link_impl(crate_name, crate_ver, Some(&search))
		} else if RUST_PRIMITIVES.contains(&first.as_str()) {
			self.primitive_link(first.as_str())
		} else {
			let (crate_name, crate_ver) = input
				.dependencies
				.get(&first)
				.map(Dependency::as_tuple)
				.unwrap_or((&first, None));
			self.build_link_impl(crate_name, crate_ver, None)
		}
	}

	fn build_link_impl(
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

	fn std_link(&self, search: &str) -> String {
		format!("https://doc.rust-lang.org/stable/std/?search={search}")
	}

	fn primitive_link(&self, primitive: &str) -> String {
		format!("https://doc.rust-lang.org/stable/std/primitive.{primitive}.html")
	}
}
