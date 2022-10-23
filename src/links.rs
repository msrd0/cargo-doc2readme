use crate::{
	depinfo::DependencyInfo,
	input::{Dependency, InputFile, LinkType}
};
use either::Either;
use syn::Path;

pub struct Links {
	pub deps: DependencyInfo
}

impl Links {
	pub fn new(template: &str, rustdoc: &str) -> Self {
		Self {
			deps: DependencyInfo::new(template, rustdoc)
		}
	}

	/// Build a link for an already fully resolved path. This method assumes that the
	/// first part of the path is the crate the path comes from.
	pub fn build_link(
		&mut self,
		path: &Path,
		link_type: Option<LinkType>,
		input: &InputFile
	) -> String {
		let mut first = path
			.segments
			.first()
			.map(|segment| segment.ident.to_string())
			.unwrap_or_default();
		let mut segments = path
			.segments
			.iter()
			.skip(1)
			.filter_map(|segment| match segment.ident.to_string() {
				ident if ident == "crate" => None,
				ident => Some(ident)
			})
			.collect::<Vec<_>>();

		// resolve crate:: and self:: links
		if (first == "crate" || first == "self") && path.leading_colon.is_none() {
			first = input.crate_name.replace('-', "_");
		}

		// get base url based on first segment
		let base_url = match first.as_str() {
			"alloc" | "core" | "proc_macro" | "std" | "test" => {
				format!("https://doc.rust-lang.org/stable/{first}")
			},
			_ => {
				let (crate_name, crate_ver) = input
					.dependencies
					.get(&first)
					.map(Dependency::as_tuple)
					.unwrap_or((&first, None));
				let lib_name = crate_name.replace('-', "_");
				self.deps.add_dependency(
					crate_name.to_owned(),
					crate_ver.cloned(),
					lib_name.clone()
				);
				if segments.is_empty() {
					format!(
						"https://crates.io/crates/{crate_name}{}",
						crate_ver.map(|ver| format!("/{ver}")).unwrap_or_default()
					)
				} else {
					format!(
						"https://docs.rs/{crate_name}/{}/{lib_name}",
						crate_ver
							.map(Either::Left)
							.unwrap_or(Either::Right("latest"))
					)
				}
			}
		};

		// get the last segment if possible
		if segments.is_empty() {
			return base_url;
		}
		let last = segments.remove(segments.len() - 1);

		// best-effort link generation
		let mut segments_uri = segments.join("/");
		if !segments_uri.is_empty() {
			segments_uri += "/";
		}
		match link_type {
			Some(LinkType::Const) => {
				format!("{base_url}/{segments_uri}constant.{last}.html")
			},
			Some(LinkType::Enum) => {
				format!("{base_url}/{segments_uri}enum.{last}.html")
			},
			Some(LinkType::Macro) => {
				format!("{base_url}/{segments_uri}macro.{last}.html")
			},
			Some(LinkType::Mod) => {
				format!("{base_url}/{segments_uri}{last}/index.html")
			},
			Some(LinkType::Primitive) => {
				format!("{base_url}/{segments_uri}primitive.{last}.html")
			},
			Some(LinkType::Static) => {
				format!("{base_url}/{segments_uri}static.{last}.html")
			},
			Some(LinkType::Struct) => {
				format!("{base_url}/{segments_uri}struct.{last}.html")
			},
			Some(LinkType::Trait) => {
				format!("{base_url}/{segments_uri}trait.{last}.html")
			},
			Some(LinkType::Type) => {
				format!("{base_url}/{segments_uri}type.{last}.html")
			},

			_ => {
				segments.push(last);
				format!("{base_url}/?search={}", segments.join("::"))
			}
		}
	}
}

#[cfg(test)]
mod tests {
	macro_rules! tests {
		($($test:ident($input:literal, $($link_type:ident ,)? $expected:literal);)*) => {
			$(
				#[test]
				fn $test() {
					let mut links = super::Links::new("", "");
					let mut input = crate::input::InputFile {
						crate_name: "my-crate".into(),
						target_type: crate::input::TargetType::Lib,
						repository: None,
						license: None,
						rust_version: None,
						rustdoc: String::new(),
						dependencies: Default::default(),
						scope: crate::input::Scope::prelude(cargo_metadata::Edition::E2021)
					};
					input.dependencies.insert(
						"my_crate".into(),
						crate::input::Dependency::new(
							"my-crate".into(),
							[semver::Comparator {
								op: semver::Op::Exact,
								major: 1,
								minor: Some(2),
								patch: Some(3),
								pre: semver::Prerelease::EMPTY
							}].into_iter().collect(),
							"1.2.3".parse().unwrap()
						)
					);

					#[allow(path_statements)]
					let input_link_type = {
						None::<crate::input::LinkType>
						$(; Some(crate::input::LinkType::$link_type))?
					};
					let href = input.scope.resolve_impl(&input.crate_name, input_link_type, $input.into());
					let path = href.path;
					let link_type = match href.link_type {
						Some(link_type) => Some(link_type),
						None => input_link_type
					};
					println!("path={path:?}");
					println!("link_type={link_type:?}");

					assert_eq!(
						links.build_link(
							&syn::parse_str::<syn::Path>(&path).unwrap(),
							link_type,
							&input
						),
						$expected
					);
				}
			)*
		};
	}

	tests! {
		test_const(
			"std::u8::MAX", Const,
			"https://doc.rust-lang.org/stable/std/u8/constant.MAX.html"
		);

		test_enum(
			"Option",
			"https://doc.rust-lang.org/stable/std/option/enum.Option.html"
		);

		test_enum_generics(
			"Option<String>",
			"https://doc.rust-lang.org/stable/std/option/enum.Option.html"
		);

		test_enum_nested_generics(
			"Option<HashMap<String, Vec<String>>>",
			"https://doc.rust-lang.org/stable/std/option/enum.Option.html"
		);

		test_macro_with(
			"vec!",
			"https://doc.rust-lang.org/stable/std/macro.vec.html"
		);

		test_macro_without(
			"vec",
			"https://doc.rust-lang.org/stable/std/macro.vec.html"
		);

		test_mod(
			"std::u8", Mod,
			"https://doc.rust-lang.org/stable/std/u8/index.html"
		);

		test_static(
			"crate::MY_STATIC", Static,
			"https://docs.rs/my-crate/1.2.3/my_crate/static.MY_STATIC.html"
		);

		test_primitive(
			"u8",
			"https://doc.rust-lang.org/stable/std/primitive.u8.html"
		);

		test_struct(
			"String",
			"https://doc.rust-lang.org/stable/std/string/struct.String.html"
		);

		test_trait(
			"Clone",
			"https://doc.rust-lang.org/stable/std/clone/trait.Clone.html"
		);

		test_trait_fn(
			"Clone::clone",
			"https://doc.rust-lang.org/stable/std/?search=clone::Clone::clone"
		);

		test_type(
			"std::ffi::c_char", Type,
			"https://doc.rust-lang.org/stable/std/ffi/type.c_char.html"
		);
	}
}
