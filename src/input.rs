use cargo::core::{Manifest, Registry, Summary};
use semver::Version;
use std::{collections::HashMap, fs::File, io::Read, path::Path};
use syn::{Attribute, Lit, LitStr, Meta};
use unindent::Unindent;

#[derive(Debug)]
pub struct InputFile {
	/// The unmodified rustdoc string
	pub rustdoc: String,
	/// The crate-level dependencies, mapping the name in rust code to the (possibly renamed)
	/// crate name and version.
	pub dependencies: HashMap<String, (String, Version)>
}

pub fn read_file<P: AsRef<Path>>(manifest: &Manifest, registry: &mut dyn Registry, path: P) -> anyhow::Result<InputFile> {
	let rustdoc = read_rustdoc_from_file(path)?;
	let dependencies = resolve_dependencies(manifest, registry)?;

	Ok(InputFile { rustdoc, dependencies })
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
