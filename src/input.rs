use cargo::core::{Edition, Manifest, Registry, Summary};
use semver::Version;
use std::{collections::HashMap, fs::File, io::Read, path::Path};
use syn::{Attribute, Lit, LitStr, Meta};
use unindent::Unindent;

#[derive(Debug)]
pub struct Scope {
	// use statements. maps name to path.
	pub uses: HashMap<String, String>,
	// items defined in the scope.
	pub items: Vec<String>
}

impl Scope {
	/// Create a new scope from the Rust prelude.
	pub fn prelude(edition: Edition) -> Self {
		let mut scope = Scope {
			// https://doc.rust-lang.org/stable/std/prelude/index.html#prelude-contents
			uses: (&[
				("Copy", "marker"),
				("Send", "marker"),
				("Sized", "marker"),
				("Sync", "marker"),
				("Unpin", "marker"),
				("Drop", "ops"),
				("Fn", "ops"),
				("FnMut", "ops"),
				("FnOnce", "ops"),
				("drop", "mem"),
				("Box", "boxed"),
				("ToOwned", "borrow"),
				("Clone", "clone"),
				("PartialEq", "cmp"),
				("PartialOrd", "cmp"),
				("Eq", "cmp"),
				("Ord", "cmp"),
				("AsRef", "convert"),
				("AsMut", "convert"),
				("Into", "convert"),
				("From", "convert"),
				("Default", "default"),
				("Iterator", "iter"),
				("Extend", "iter"),
				("IntoIterator", "iter"),
				("DoubleEndedIterator", "iter"),
				("ExactSizeIterator", "iter"),
				("Option", "option"),
				("Some", "option::Option"),
				("None", "option::Option"),
				("Result", "result"),
				("Ok", "result::Result"),
				("Err", "result::Result"),
				("String", "string"),
				("ToString", "string"),
				("Vec", "vec")
			])
				.iter()
				.map(|(name, path)| (name.to_string(), format!("::std::{}::{}", path, name)))
				.collect(),
			items: Vec::new()
		};

		if edition >= Edition::Edition2021 {
			// https://blog.rust-lang.org/2021/05/11/edition-2021.html#additions-to-the-prelude
			scope.uses.insert("TryInto".to_owned(), "::std::convert::TryInto".to_owned());
			scope.uses.insert("TryFrom".to_owned(), "::std::convert::TryFrom".to_owned());
			scope
				.uses
				.insert("FromIterator".to_owned(), "::std::iter::FromIterator".to_owned());
		}

		scope
	}
}

#[derive(Debug)]
pub struct InputFile {
	/// The name of the crate.
	pub crate_name: String,
	/// The repository url (if specified).
	pub repository: Option<String>,
	/// The license field (if specified).
	pub license: Option<String>,
	/// The unmodified rustdoc string
	pub rustdoc: String,
	/// The crate-level dependencies, mapping the name in rust code to the (possibly renamed)
	/// crate name and version.
	pub dependencies: HashMap<String, (String, Version)>,
	/// The scope at the crate root.
	pub scope: Scope
}

pub fn read_file<P: AsRef<Path>>(manifest: &Manifest, registry: &mut dyn Registry, path: P) -> anyhow::Result<InputFile> {
	let crate_name = manifest.name().to_string();
	let repository = manifest.metadata().repository.clone();
	let license = manifest.metadata().license.clone();

	let rustdoc = read_rustdoc_from_file(path)?;
	let dependencies = resolve_dependencies(manifest, registry)?;

	// TODO
	let scope = Scope::prelude(manifest.edition());

	Ok(InputFile {
		crate_name,
		repository,
		license,
		rustdoc,
		dependencies,
		scope
	})
}

fn read_rustdoc_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<String> {
	let mut file = File::open(path)?;
	let mut buf = String::new();
	file.read_to_string(&mut buf)?;
	let file = syn::parse_file(&buf)?;

	let mut doc = String::new();
	for attr in file.attrs {
		if attr.path.is_ident("doc") {
			if let Some(str) = parse_doc_attr(&attr)? {
				// always push a newline: unindent ignores the first line
				doc.push('\n');
				doc.push_str(&str.value());
			}
		}
	}
	Ok(doc.unindent())
}

fn parse_doc_attr(input: &Attribute) -> syn::Result<Option<LitStr>> {
	input.parse_meta().and_then(|meta| {
		Ok(match meta {
			Meta::NameValue(kv) => Some(match kv.lit {
				Lit::Str(str) => str,
				lit => return Err(syn::Error::new(lit.span(), "Expected string literal"))
			}),
			_ => None
		})
	})
}

fn resolve_dependencies(
	manifest: &Manifest,
	registry: &mut dyn Registry
) -> anyhow::Result<HashMap<String, (String, Version)>> {
	let mut deps = HashMap::new();

	for dep in manifest.dependencies() {
		let mut f = |sum: Summary| {
			if deps
				.get(dep.name_in_toml().as_str())
				.map(|(_, ver)| ver < sum.version())
				.unwrap_or(true)
			{
				deps.insert(
					dep.name_in_toml().to_string(),
					(sum.name().to_string(), sum.version().clone())
				);
			}
		};
		registry.query(dep, &mut f, false).expect("Failed to resolve dependency");
	}

	Ok(deps)
}
