use anyhow::{bail, Context};
use cargo::core::{Edition, Manifest, Registry, Summary, Target, TargetKind};
use semver::Version;
use std::{
	collections::HashMap,
	fs::File,
	io::{self, Read, Write},
	path::Path,
	process::{Command, Output},
	task::Poll
};
use syn::{Attribute, Item, ItemUse, Lit, LitStr, Meta, UsePath, UseTree};
use unindent::Unindent;

#[derive(Debug)]
pub struct Scope {
	// use statements and declared items. maps name to path.
	pub scope: HashMap<String, String>,
	// the scope included a wildcard use statement.
	pub has_glob_use: bool
}

impl Scope {
	fn insert<K, V>(&mut self, key: K, value: V)
	where
		K: Into<String>,
		V: Into<String>
	{
		self.scope.insert(key.into(), value.into());
	}

	/// Create a new scope from the Rust prelude.
	pub fn prelude(edition: Edition) -> Self {
		let mut scope = Scope {
			// https://doc.rust-lang.org/stable/std/prelude/index.html#prelude-contents
			scope: [
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
			]
			.into_iter()
			.map(|(name, path)| (name.into(), format!("::std::{path}::{name}")))
			.collect(),
			has_glob_use: false
		};

		if edition >= Edition::Edition2021 {
			// https://blog.rust-lang.org/2021/05/11/edition-2021.html#additions-to-the-prelude
			scope.insert("TryInto", "::std::convert::TryInto");
			scope.insert("TryFrom", "::std::convert::TryFrom");
			scope.insert("FromIterator", "::std::iter::FromIterator");
		}

		scope
	}
}

#[derive(Debug)]
pub struct CrateCode(String);

impl CrateCode {
	pub(crate) fn read_from_disk<P>(path: &P) -> anyhow::Result<CrateCode>
	where
		P: AsRef<Path> + ?Sized
	{
		let mut file = File::open(path)?;
		let mut buf = String::new();
		file.read_to_string(&mut buf)?;

		Ok(CrateCode(buf))
	}

	pub(crate) fn read_expansion<P>(manifest_path: &P, target: &Target) -> anyhow::Result<CrateCode>
	where
		P: AsRef<Path> + ?Sized
	{
		let mut cmd = Command::new("cargo");
		cmd.arg("+nightly").arg("rustc").arg(format!(
			"--manifest-path={}",
			manifest_path.as_ref().display()
		));
		match target.kind() {
			TargetKind::Lib(_) => {
				cmd.arg("--lib");
			},
			TargetKind::Bin => {
				cmd.arg("--bin").arg(target.name());
			},
			_ => {}
		};
		cmd.arg("--").arg("-Zunpretty=expanded");

		let Output {
			stdout,
			stderr,
			status
		} = cmd.output()
			.context("Failed to run cargo to expand crate content")?;

		if !status.success() {
			// Something bad happened during the compilation. Let's print
			// anything Cargo reported to us and return an error.
			io::stdout()
				.lock()
				.write_all(stderr.as_slice())
				.expect("Failed to write cargo errors to stdout");

			bail!("Cargo failed to expand the macros")
		}

		String::from_utf8(stdout)
			.context("Failed to convert cargo output to UTF-8")
			.map(CrateCode)
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
	/// The crate-level dependencies, mapping the valid identifier in rust code to the (possibly
	/// renamed, containing invalid characters, etc.) crate name and version.
	pub dependencies: HashMap<String, (String, Version)>,
	/// The scope at the crate root.
	pub scope: Scope
}

pub fn read_code(
	manifest: &Manifest,
	registry: &mut dyn Registry,
	code: CrateCode
) -> anyhow::Result<InputFile> {
	let crate_name = manifest.name().to_string();
	let repository = manifest.metadata().repository.clone();
	let license = manifest.metadata().license.clone();

	let file = syn::parse_file(code.0.as_str())?;

	let rustdoc = read_rustdoc_from_file(&file)?;
	let dependencies = resolve_dependencies(manifest, registry)?;
	let scope = read_scope_from_file(manifest, &file)?;

	Ok(InputFile {
		crate_name,
		repository,
		license,
		rustdoc,
		dependencies,
		scope
	})
}

fn read_rustdoc_from_file(file: &syn::File) -> anyhow::Result<String> {
	let mut doc = String::new();
	for attr in &file.attrs {
		if attr.path.is_ident("doc") {
			if let Some(str) = parse_doc_attr(attr)? {
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

	// we currently insert our own crate as a dependency to allow doc links referencing ourself.
	// however, we might want to change this so that custom doc urls can be used, so that e.g.
	// the repository readme points to some rustdoc generated from the master branch, instead of
	// the last release. Also, the version in the current manifest might not be released, so
	// those links might be dead.
	deps.insert(
		manifest.name().to_string().replace('-', "_"),
		(manifest.name().to_string(), manifest.version().clone())
	);

	let pending_deps = manifest
		.dependencies()
		.iter()
		.map(|dep| {
			let dep_name = dep.name_in_toml().to_string().replace('-', "_");
			let mut f = |sum: Summary| {
				if deps
					.get(&dep_name)
					.map(|(_, ver)| ver < sum.version())
					.unwrap_or(true)
				{
					deps.insert(
						dep_name.clone(),
						(sum.name().to_string(), sum.version().clone())
					);
				}
			};
			registry.query(dep, &mut f, false)
		})
		.collect::<Vec<_>>();
	registry
		.block_until_ready()
		.expect("Failed to wait for dependency resolver");
	for dep in pending_deps {
		match dep {
			Poll::Ready(dep) => dep.expect("Failed to resolve dependency"),
			_ => unreachable!("We've waited for the dependency to be ready")
		}
	}

	Ok(deps)
}

macro_rules! item_ident {
	($crate_name:expr, $ident:expr) => {{
		let ident: &::syn::Ident = $ident;
		(ident, format!("::{}::{ident}", $crate_name))
	}};
}

fn read_scope_from_file(manifest: &Manifest, file: &syn::File) -> anyhow::Result<Scope> {
	let crate_name = manifest.name().replace('-', "_");
	let mut scope = Scope::prelude(manifest.edition());

	for i in &file.items {
		let mut is_macro = false;
		let (ident, path) = match i {
			Item::Const(i) => item_ident!(crate_name, &i.ident),
			Item::Enum(i) => item_ident!(crate_name, &i.ident),
			Item::ExternCrate(i) if i.ident != "self" && i.rename.is_some() => {
				(&i.rename.as_ref().unwrap().1, format!("::{}", i.ident))
			},
			Item::Fn(i) => item_ident!(crate_name, &i.sig.ident),
			Item::Macro(i) if i.ident.is_some() => {
				is_macro = true;
				item_ident!(crate_name, i.ident.as_ref().unwrap())
			},
			Item::Macro2(i) => {
				is_macro = true;
				item_ident!(crate_name, &i.ident)
			},
			Item::Mod(i) => item_ident!(crate_name, &i.ident),
			Item::Static(i) => item_ident!(crate_name, &i.ident),
			Item::Struct(i) => item_ident!(crate_name, &i.ident),
			Item::Trait(i) => item_ident!(crate_name, &i.ident),
			Item::TraitAlias(i) => item_ident!(crate_name, &i.ident),
			Item::Type(i) => item_ident!(crate_name, &i.ident),
			Item::Union(i) => item_ident!(crate_name, &i.ident),
			Item::Use(i) if !is_prelude_import(i) => {
				add_use_tree_to_scope(&mut scope, String::new(), &i.tree);
				continue;
			},
			_ => continue
		};
		if is_macro {
			scope.insert(format!("{ident}!"), path.clone());
		}
		scope.insert(ident.to_string(), path);
	}

	Ok(scope)
}

fn add_use_tree_to_scope(scope: &mut Scope, prefix: String, tree: &UseTree) {
	match tree {
		UseTree::Path(path) => {
			add_use_tree_to_scope(scope, format!("{prefix}{}::", path.ident), &path.tree)
		},
		UseTree::Name(name) => {
			// skip `pub use dependency;` style uses; they don't add any unknown elements to the scope
			if !prefix.is_empty() {
				scope.insert(name.ident.to_string(), format!("{prefix}{}", name.ident));
			}
		},
		UseTree::Rename(name) => {
			scope.insert(name.rename.to_string(), format!("{prefix}{}", name.ident));
		},
		UseTree::Glob(_) => {
			scope.has_glob_use = true;
		},
		UseTree::Group(group) => {
			for tree in &group.items {
				add_use_tree_to_scope(scope, prefix.clone(), tree);
			}
		},
	};
}

fn is_prelude_import(item_use: &ItemUse) -> bool {
	match &item_use.tree {
		UseTree::Path(UsePath { ident, tree, .. }) if ident == "std" => match tree.as_ref() {
			UseTree::Path(UsePath { ident, .. }) => ident == "prelude",
			_ => false
		},
		_ => false
	}
}
