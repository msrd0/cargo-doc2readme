use anyhow::{bail, Context};
use cargo_metadata::{Edition, Metadata, Package, Target};
use log::{info, warn};
use semver::{Comparator, Op, Version, VersionReq};
use std::{
	collections::{HashMap, HashSet, VecDeque},
	fmt::{self, Debug, Formatter},
	fs::File,
	io::{self, Read, Write},
	path::Path,
	process::{Command, Output}
};
use syn::{Attribute, Ident, Item, ItemUse, Lit, LitStr, Meta, UsePath, UseTree, Visibility};
use unindent::Unindent;

type ScopeScope = HashMap<String, VecDeque<(LinkType, String)>>;

#[derive(Debug)]
pub struct Scope {
	// use statements and declared items. maps name to path.
	pub scope: ScopeScope,
	// private modules so that `pub use`'d items are considered inlined.
	pub privmods: HashSet<String>,
	// the scope included a wildcard use statement.
	pub has_glob_use: bool
}

#[derive(Debug, Eq, PartialEq)]
pub enum LinkType {
	/// Function statements.
	Function,

	/// `use` statement that links to a path.
	Use,

	/// `pub use` statement that links to the name pub used as.
	PubUse,

	/// Other statements we'll implement later.
	Other
}

fn make_prelude<const N: usize>(prelude: [(&'static str, &'static str); N]) -> ScopeScope {
	prelude
		.into_iter()
		.map(|(name, path)| {
			(
				name.into(),
				[(LinkType::Use, format!("::std::{path}::{name}"))]
					.into_iter()
					.collect()
			)
		})
		.collect()
}

impl Scope {
	fn insert<K, V>(&mut self, key: K, ty: LinkType, value: V)
	where
		K: Into<String>,
		V: Into<String>
	{
		self.scope
			.entry(key.into())
			.or_insert_with(VecDeque::new)
			.push_front((ty, value.into()));
	}

	/// Create a new scope from the Rust prelude.
	pub fn prelude(edition: Edition) -> Self {
		let mut scope = Scope {
			// https://doc.rust-lang.org/stable/std/prelude/index.html#prelude-contents
			scope: make_prelude([
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
			]),
			privmods: HashSet::new(),
			has_glob_use: false
		};

		if edition >= Edition::E2021 {
			// https://blog.rust-lang.org/2021/05/11/edition-2021.html#additions-to-the-prelude
			for (key, value) in make_prelude([
				("TryInto", "convert"),
				("TryFrom", "convert"),
				("FromIterator", "iter")
			]) {
				scope.scope.insert(key, value);
			}
		}

		scope
	}
}

#[derive(Debug)]
pub struct CrateCode(String);

impl CrateCode {
	pub(crate) fn read_from_disk<P>(path: P) -> anyhow::Result<CrateCode>
	where
		P: AsRef<Path>
	{
		let mut file = File::open(path)?;
		let mut buf = String::new();
		file.read_to_string(&mut buf)?;

		Ok(CrateCode(buf))
	}

	pub(crate) fn read_expansion<P>(
		manifest_path: Option<P>,
		target: &Target
	) -> anyhow::Result<CrateCode>
	where
		P: AsRef<Path>
	{
		let mut cmd = Command::new("cargo");
		cmd.arg("+nightly").arg("rustc");
		if let Some(manifest_path) = manifest_path {
			cmd.arg("--manifest-path").arg(manifest_path.as_ref());
		}
		if target.kind.iter().any(|kind| kind == "lib") {
			cmd.arg("--lib");
		} else if target.kind.iter().any(|kind| kind == "bin") {
			cmd.arg("--bin").arg(&target.name);
		}
		cmd.arg("--").arg("-Zunpretty=expanded");

		info!("Running rustc -Zunpretty=expanded");
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
	/// The rust_version field (if specified).
	pub rust_version: Option<VersionReq>,
	/// The unmodified rustdoc string
	pub rustdoc: String,
	/// The crate-level dependencies, mapping the valid identifier in rust code to the (possibly
	/// renamed, containing invalid characters, etc.) crate name and version.
	pub dependencies: HashMap<String, Dependency>,
	/// The scope at the crate root.
	pub scope: Scope
}

pub struct Dependency {
	/// The crate name as it appears on crates.io.
	pub crate_name: String,

	/// The version requirement of the dependency.
	pub req: VersionReq,

	/// The exact version of the dependency.
	pub version: Version
}

impl Dependency {
	fn new(crate_name: String, req: VersionReq, version: Version) -> Self {
		Self {
			crate_name,
			req,
			version
		}
	}

	pub fn as_tuple(&self) -> (&str, Option<&Version>) {
		(self.crate_name.as_str(), Some(&self.version))
	}
}

impl Debug for Dependency {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{} = \"{}\" ({})",
			self.crate_name, self.req, self.version
		)
	}
}

pub fn read_code(metadata: &Metadata, pkg: &Package, code: CrateCode) -> anyhow::Result<InputFile> {
	let crate_name = pkg.name.clone();
	let repository = pkg.repository.clone();
	let license = pkg.license.clone();
	let rust_version = pkg.rust_version.clone();

	let file = syn::parse_file(code.0.as_str())?;

	let rustdoc = read_rustdoc_from_file(&file)?;
	let dependencies = resolve_dependencies(metadata, pkg)?;
	let scope = read_scope_from_file(pkg, &file)?;

	Ok(InputFile {
		crate_name,
		repository,
		license,
		rust_version,
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

fn sanitize_crate_name<T: AsRef<str>>(name: T) -> String {
	name.as_ref().replace('-', "_")
}

fn resolve_dependencies(
	metadata: &Metadata,
	pkg: &Package
) -> anyhow::Result<HashMap<String, Dependency>> {
	let mut deps = HashMap::new();

	// we currently insert our own crate as a dependency to allow doc links referencing ourself.
	// however, we might want to change this so that custom doc urls can be used, so that e.g.
	// the repository readme points to some rustdoc generated from the master branch, instead of
	// the last release. Also, the version in the current manifest might not be released, so
	// those links might be dead.
	let version = pkg.version.clone();
	deps.insert(
		sanitize_crate_name(&pkg.name),
		Dependency::new(
			pkg.name.clone(),
			[Comparator {
				op: Op::Exact,
				major: version.major,
				minor: Some(version.minor),
				patch: Some(version.patch),
				pre: version.pre.clone()
			}]
			.into_iter()
			.collect(),
			version
		)
	);

	for dep in &pkg.dependencies {
		let dep_name = sanitize_crate_name(&dep.name);
		let version = metadata
			.packages
			.iter()
			.find(|pkg| pkg.name == dep.name)
			.map(|pkg| &pkg.version);
		let rename = dep.rename.as_ref().unwrap_or(&dep_name);

		if let Some(version) = version {
			if deps
				.get(&dep_name)
				.map(|dep| dep.version < *version)
				.unwrap_or(true)
			{
				deps.insert(
					rename.to_owned(),
					Dependency::new(dep_name, dep.req.clone(), version.to_owned())
				);
			}
		} else {
			warn!("Unable to find version of dependency {}", dep.name);
		}
	}

	Ok(deps)
}

macro_rules! scope_insert {
	($scope:ident, $crate_name:expr, $ident:expr) => {{
		let ident: &::syn::Ident = $ident;
		$scope.insert(
			ident.to_string(),
			LinkType::Other,
			format!("::{}::{ident}", $crate_name)
		);
	}};
}

fn read_scope_from_file(pkg: &Package, file: &syn::File) -> anyhow::Result<Scope> {
	let crate_name = sanitize_crate_name(&pkg.name);
	let mut scope = Scope::prelude(pkg.edition.clone());

	for i in &file.items {
		match i {
			Item::Const(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::Enum(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::ExternCrate(i) if i.ident != "self" && i.rename.is_some() => {
				let krate = &i.rename.as_ref().unwrap().1;
				scope.insert(krate.to_string(), LinkType::Other, format!("::{}", i.ident));
			},
			Item::Fn(i) => add_fun_to_scope(&mut scope, &crate_name, &i.sig.ident),
			Item::Macro(i) if i.ident.is_some() => {
				add_macro_to_scope(&mut scope, &crate_name, i.ident.as_ref().unwrap())
			},
			Item::Macro2(i) => add_macro_to_scope(&mut scope, &crate_name, &i.ident),
			Item::Mod(i) => match i.vis {
				Visibility::Public(_) => {
					scope_insert!(scope, crate_name, &i.ident)
				},
				_ => {
					scope.privmods.insert(i.ident.to_string());
				}
			},
			Item::Static(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::Struct(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::Trait(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::TraitAlias(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::Type(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::Union(i) => scope_insert!(scope, crate_name, &i.ident),
			Item::Use(i) if !is_prelude_import(i) => {
				add_use_tree_to_scope(&mut scope, &crate_name, &i.vis, String::new(), &i.tree)
			},
			_ => {}
		};
	}

	// remove privmod imports from scope
	for values in &mut scope.scope.values_mut() {
		let mut i = 0;
		while i < values.len() {
			if values[i].0 == LinkType::Use {
				let path = &values[i].1;
				if (!path.starts_with("::") || path.starts_with(&format!("::{crate_name}::")))
					&& Some(path.split("::").collect::<Vec<_>>())
						.map(|segments| segments.len() > 1 && scope.privmods.contains(segments[0]))
						.unwrap()
				{
					values.remove(i);
					continue;
				}
			}

			i += 1;
		}
	}

	Ok(scope)
}

fn add_fun_to_scope(scope: &mut Scope, crate_name: &str, ident: &Ident) {
	let path = format!("::{crate_name}::{ident}");
	scope.insert(ident.to_string(), LinkType::Function, &path);
	scope.insert(format!("{ident}()"), LinkType::Function, &path);
}

fn add_macro_to_scope(scope: &mut Scope, crate_name: &str, ident: &Ident) {
	let path = format!("::{crate_name}::{ident}");
	scope.insert(ident.to_string(), LinkType::Other, &path);
	scope.insert(format!("{ident}!"), LinkType::Other, &path)
}

fn add_use_tree_to_scope(
	scope: &mut Scope,
	crate_name: &str,
	vis: &Visibility,
	prefix: String,
	tree: &UseTree
) {
	match tree {
		UseTree::Path(path) => add_use_tree_to_scope(
			scope,
			crate_name,
			vis,
			format!("{prefix}{}::", path.ident),
			&path.tree
		),
		UseTree::Name(name) => {
			// skip `pub use dependency;` style uses; they don't add any unknown elements to the scope
			if !prefix.is_empty() {
				add_use_item_to_scope(scope, crate_name, vis, &prefix, &name.ident, &name.ident);
			}
		},
		UseTree::Rename(name) => {
			add_use_item_to_scope(scope, crate_name, vis, &prefix, &name.rename, &name.ident);
		},
		UseTree::Glob(_) => {
			scope.has_glob_use = true;
		},
		UseTree::Group(group) => {
			for tree in &group.items {
				add_use_tree_to_scope(scope, crate_name, vis, prefix.clone(), tree);
			}
		},
	};
}

fn add_use_item_to_scope(
	scope: &mut Scope,
	crate_name: &str,
	vis: &Visibility,
	prefix: &str,
	rename: &Ident,
	ident: &Ident
) {
	if matches!(vis, Visibility::Public(_)) {
		scope.insert(
			rename.to_string(),
			LinkType::PubUse,
			format!("::{crate_name}::{rename}")
		);
	}
	scope.insert(
		rename.to_string(),
		LinkType::Use,
		format!("{prefix}{ident}")
	);
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
