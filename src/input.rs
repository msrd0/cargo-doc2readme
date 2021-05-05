use std::{fs::File, io::Read, path::Path};
use syn::{Attribute, Lit, LitStr, Meta};
use unindent::Unindent;

pub fn read_file<P: AsRef<Path>>(path: P) -> anyhow::Result<String> {
	println!("Reading file {}", path.as_ref().display());
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
