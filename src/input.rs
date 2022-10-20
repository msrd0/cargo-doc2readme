use crate::{diagnostic::Diagnostic, preproc::Preprocessor};
use anyhow::{bail, Context};
use cargo_metadata::{Edition, Metadata, Package, Target};
use log::{debug, info};
use semver::{Comparator, Op, Version, VersionReq};
use std::{
	collections::{HashMap, HashSet, VecDeque},
	fmt::{self, Debug, Formatter},
	fs::File,
	io::{self, BufReader, Cursor, Read, Write},
	path::Path,
	process::{Command, Output}
};
use syn::{Attribute, Ident, Item, ItemUse, Lit, LitStr, Meta, UsePath, UseTree, Visibility};

type ScopeScope = HashMap<String, VecDeque<(LinkType, String)>>;

#[derive(Debug)]
pub struct Scope {
	// use statements and declared items. maps name to path.
	pub scope: ScopeScope,
	// private modules so that `pub use`'d items are considered inlined.
	pub privmods: HashSet<String>
}

#[derive(Debug, Eq, PartialEq)]
pub enum LinkType {
	Const,
	Enum,
	ExternCrate,
	Function,
	Macro,
	Mod,
	Static,
	Struct,
	Trait,
	TraitAlias,
	Type,
	Union,

	/// `use` statement that links to a path.
	Use,
	/// `pub use` statement that links to the name pub used as.
	PubUse
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
			privmods: HashSet::new()
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
pub struct CrateCode(pub String);

impl CrateCode {
	fn read_from<R>(read: R) -> io::Result<Self>
	where
		R: io::BufRead
	{
		let mut preproc = Preprocessor::new(read);
		let mut buf = String::new();
		preproc.read_to_string(&mut buf)?;

		Ok(Self(buf))
	}

	pub fn read_from_disk<P>(path: P) -> io::Result<Self>
	where
		P: AsRef<Path>
	{
		Self::read_from(BufReader::new(File::open(path)?))
	}

	pub fn read_expansion<P>(manifest_path: Option<P>, target: &Target) -> anyhow::Result<CrateCode>
	where
		P: AsRef<Path>
	{
		let mut cmd = Command::new("cargo");
		cmd.arg("+nightly").arg("rustc");
		if let Some(manifest_path) = manifest_path {
			cmd.arg("--manifest-path").arg(manifest_path.as_ref());
		}
		if target.is_lib() {
			cmd.arg("--lib");
		} else if target.is_bin() {
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

		Ok(Self::read_from(Cursor::new(stdout))?)
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

pub fn read_code(
	metadata: &Metadata,
	pkg: &Package,
	code: CrateCode,
	diagnostics: &mut Diagnostic
) -> InputFile {
	let crate_name = pkg.name.clone();
	let repository = pkg.repository.clone();
	let license = pkg.license.clone();
	let rust_version = pkg.rust_version.clone();

	debug!("Reading code \n{}", code.0);
	let file = match syn::parse_file(code.0.as_str()) {
		Ok(file) => file,
		Err(err) => {
			diagnostics.syntax_error(err);
			syn::parse_file("").unwrap()
		}
	};

	let rustdoc = match read_rustdoc_from_file(&file) {
		Ok(rustdoc) => rustdoc,
		Err(err) => {
			diagnostics.syntax_error(err);
			String::new()
		}
	};

	let dependencies = resolve_dependencies(metadata, pkg, diagnostics);
	let scope = read_scope_from_file(pkg, &file, diagnostics);

	InputFile {
		crate_name,
		repository,
		license,
		rust_version,
		rustdoc,
		dependencies,
		scope
	}
}

fn read_rustdoc_from_file(file: &syn::File) -> syn::Result<String> {
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
	Ok(doc)
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
	pkg: &Package,
	diagnostics: &mut Diagnostic
) -> HashMap<String, Dependency> {
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
			diagnostics.warn(format!("Unable to find version of dependency {}", dep.name));
		}
	}

	deps
}

struct ScopeEditor<'a> {
	scope: &'a mut Scope,
	crate_name: &'a str,
	diagnostics: &'a mut Diagnostic
}

impl<'a> ScopeEditor<'a> {
	fn new(scope: &'a mut Scope, crate_name: &'a str, diagnostics: &'a mut Diagnostic) -> Self {
		Self {
			scope,
			crate_name,
			diagnostics
		}
	}

	fn add_privmod(&mut self, ident: &Ident) {
		self.scope.privmods.insert(ident.to_string());
	}

	fn insert(&mut self, ident: &Ident, ty: LinkType) {
		let path = format!("::{}::{ident}", self.crate_name);
		self.scope.insert(ident.to_string(), ty, path);
	}

	fn insert_fun(&mut self, ident: &Ident) {
		let path = format!("::{}::{ident}", self.crate_name);
		self.scope
			.insert(ident.to_string(), LinkType::Function, &path);
		self.scope
			.insert(format!("{ident}()"), LinkType::Function, path);
	}

	fn insert_macro(&mut self, ident: &Ident) {
		let path = format!("::{}::{ident}", self.crate_name);
		self.scope.insert(ident.to_string(), LinkType::Macro, &path);
		self.scope
			.insert(format!("{ident}!"), LinkType::Macro, path);
	}

	fn insert_use_tree(&mut self, vis: &Visibility, tree: &UseTree) {
		self.insert_use_tree_impl(vis, String::new(), tree)
	}

	fn insert_use_tree_impl(&mut self, vis: &Visibility, prefix: String, tree: &UseTree) {
		match tree {
			UseTree::Path(path) => {
				self.insert_use_tree_impl(vis, format!("{prefix}{}::", path.ident), &path.tree)
			},
			UseTree::Name(name) => {
				// skip `pub use dependency;` style uses; they don't add any unknown
				// elements to the scope
				if !prefix.is_empty() {
					self.insert_use_item(vis, &prefix, &name.ident, &name.ident);
				}
			},
			UseTree::Rename(name) => {
				self.insert_use_item(vis, &prefix, &name.rename, &name.ident);
			},
			UseTree::Glob(glob) => {
				self.diagnostics.warn_with_label(
					"Glob use statements can lead to incomplete link generation.",
					glob.star_token.spans[0],
					"All items imported through this glob use will not be used for link generation"
				);
			},
			UseTree::Group(group) => {
				for tree in &group.items {
					self.insert_use_tree_impl(vis, prefix.clone(), tree);
				}
			},
		};
	}

	fn insert_use_item(&mut self, vis: &Visibility, prefix: &str, rename: &Ident, ident: &Ident) {
		if matches!(vis, Visibility::Public(_)) {
			self.insert(rename, LinkType::PubUse);
		}
		self.scope.insert(
			rename.to_string(),
			LinkType::Use,
			format!("{prefix}{ident}")
		);
	}
}

fn read_scope_from_file(pkg: &Package, file: &syn::File, diagnostics: &mut Diagnostic) -> Scope {
	let crate_name = sanitize_crate_name(&pkg.name);
	let mut scope = Scope::prelude(pkg.edition.clone());
	let mut editor = ScopeEditor::new(&mut scope, &crate_name, diagnostics);

	for i in &file.items {
		match i {
			Item::Const(i) => editor.insert(&i.ident, LinkType::Const),
			Item::Enum(i) => editor.insert(&i.ident, LinkType::Enum),
			Item::ExternCrate(i) if i.ident != "self" && i.rename.is_some() => {
				editor.scope.insert(
					i.rename.as_ref().unwrap().1.to_string(),
					LinkType::ExternCrate,
					format!("::{}", i.ident)
				);
			},
			Item::Fn(i) => editor.insert_fun(&i.sig.ident),
			Item::Macro(i) if i.ident.is_some() => editor.insert_macro(i.ident.as_ref().unwrap()),
			Item::Macro2(i) => editor.insert_macro(&i.ident),
			Item::Mod(i) if matches!(i.vis, Visibility::Public(_)) => {
				editor.insert(&i.ident, LinkType::Mod)
			},
			Item::Mod(i) => editor.add_privmod(&i.ident),
			Item::Static(i) => editor.insert(&i.ident, LinkType::Static),
			Item::Struct(i) => editor.insert(&i.ident, LinkType::Struct),
			Item::Trait(i) => editor.insert(&i.ident, LinkType::Trait),
			Item::TraitAlias(i) => editor.insert(&i.ident, LinkType::TraitAlias),
			Item::Type(i) => editor.insert(&i.ident, LinkType::Type),
			Item::Union(i) => editor.insert(&i.ident, LinkType::Union),
			Item::Use(i) if !is_prelude_import(i) => editor.insert_use_tree(&i.vis, &i.tree),
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

	scope
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
