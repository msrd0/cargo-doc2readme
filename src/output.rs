use crate::input::{InputFile, Scope};
use anyhow::anyhow;
use base64::URL_SAFE_NO_PAD;
use blake3::Hash;
use either::Either;
use itertools::Itertools;
use pulldown_cmark::{
	Alignment, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag
};
use semver::Version;
use serde::{Deserialize, Serialize, Serializer};
use std::{
	collections::{BTreeMap, BTreeSet, VecDeque},
	fmt::{self, Write as _},
	io
};
use syn::Path;
use tera::Tera;
use url::Url;

const DEFAULT_CODEBLOCK_LANG: &str = "rust";
/// List of codeblock flags that rustdoc allows
const RUSTDOC_CODEBLOCK_FLAGS: &[&str] = &[
	"compile_fail",
	"edition2015",
	"edition2018",
	"edition2021",
	"ignore",
	"no_run",
	"should_panic"
];
const RUST_PRIMITIVES: &[&str] = &[
	// https://doc.rust-lang.org/stable/std/primitive/index.html#reexports
	"bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "str", "u128", "u16",
	"u32", "u64", "u8", "usize"
];

impl Scope {
	fn resolve(&self, crate_name: &str, path: String) -> String {
		if path.starts_with("::") {
			return path;
		}
		let mut segments = path.split("::").collect::<Vec<_>>();
		if segments[0] == "crate" {
			segments[0] = crate_name;
		}
		if self.scope.contains_key(segments[0]) {
			segments[0] = &self.scope[segments[0]];
			return self.resolve(crate_name, segments.join("::"));
		}
		path
	}
}

struct HashDef;

impl HashDef {
	fn serialize<S: Serializer>(this: &Hash, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer
	{
		let hash = this.as_bytes();
		let parts = (
			u64::from_be_bytes((&hash[0..8]).try_into().unwrap()),
			u64::from_be_bytes((&hash[8..16]).try_into().unwrap()),
			u64::from_be_bytes((&hash[16..24]).try_into().unwrap()),
			u64::from_be_bytes((&hash[24..32]).try_into().unwrap())
		);
		parts.serialize(serializer)
	}
}

#[derive(Serialize)]
struct DependencyHash {
	/// The version of this dependency hash. Increase whenever the format of this struct.
	#[serde(rename = "v")]
	hash_version: u8,

	/// The version of the markdown output. If there are significant changes made to the
	/// markdown output that require to re-run this tool eventhough none of the inputs
	/// has changed, this version should be increased.
	#[serde(rename = "m")]
	markdown_version: u8,

	/// The blake3 hash of the template file.
	#[serde(rename = "t", with = "HashDef")]
	template_hash: Hash,

	/// The blake3 hash of the input rustdoc.
	#[serde(rename = "r", with = "HashDef")]
	rustdoc_hash: Hash,

	/// The versions of dependencies that are used for link generation. The first entry
	/// of the tuple is the dependency name on crates.io, the second is the version,
	/// and the third is the dependency name as seen in Rust code (or missing if it is
	/// equivalent to the dependency name on crates.io).
	#[serde(rename = "d")]
	dependencies: BTreeSet<(String, Option<Version>, Option<String>)>
}

impl DependencyHash {
	fn new(template_hash: Hash, rustdoc_hash: Hash) -> Self {
		Self {
			hash_version: 0,
			markdown_version: 0,
			template_hash: template_hash.into(),
			rustdoc_hash: rustdoc_hash.into(),
			dependencies: BTreeSet::new()
		}
	}

	fn is_empty(&self) -> bool {
		self.dependencies.is_empty()
	}

	fn add_dep(&mut self, crate_name: String, version: Option<Version>, lib_name: String) {
		self.dependencies.insert(if lib_name == crate_name {
			(crate_name, version, None)
		} else {
			(crate_name, version, Some(lib_name))
		});
	}
}

fn broken_link_callback<'a>(lnk: BrokenLink<'_>) -> Option<(CowStr<'a>, CowStr<'a>)> {
	Some(("".into(), lnk.reference.to_string().into()))
}

fn newline(out: &mut dyn fmt::Write, indent: &VecDeque<&'static str>) -> fmt::Result {
	writeln!(out)?;
	for s in indent {
		write!(out, "{s}")?;
	}
	Ok(())
}

pub fn emit(input: InputFile, template: &str, out_file: &mut dyn io::Write) -> anyhow::Result<()> {
	// we need this broken link callback for the purpose of broken links being parsed as links
	let mut broken_link_callback = broken_link_callback;
	let parser = Parser::new_with_broken_link_callback(
		&input.rustdoc,
		Options::all(),
		Some(&mut broken_link_callback)
	);

	let mut alignments: Vec<Alignment> = Vec::new();
	let mut has_newline = true;
	let mut links: BTreeMap<String, String> = BTreeMap::new();
	let mut link_idx: u32 = 0;
	let mut indent: VecDeque<&'static str> = VecDeque::new();
	let mut lists: VecDeque<Option<u64>> = VecDeque::new();

	let mut readme = String::new();
	let out = &mut readme;
	for ev in parser {
		//println!("[DEBUG] ev = {:?}", ev);
		match ev {
			Event::Start(tag) => match tag {
				Tag::Paragraph => Ok(()),
				Tag::Heading(lvl, ..) => {
					newline(out, &indent)?;
					for _ in 0..=lvl as u8 {
						write!(out, "#")?;
					}
					write!(out, " ")
				},
				Tag::BlockQuote => {
					newline(out, &indent)?;
					indent.push_back("> ");
					write!(out, "> ")
				},
				Tag::CodeBlock(CodeBlockKind::Indented) => {
					newline(out, &indent)?;
					write!(out, "```{}", DEFAULT_CODEBLOCK_LANG)?;
					newline(out, &indent)
				},
				Tag::CodeBlock(CodeBlockKind::Fenced(lang)) => {
					// Strip rustdoc code-block flags from the language
					let mut lang = lang.to_string();
					for flag in RUSTDOC_CODEBLOCK_FLAGS {
						lang = lang.replace(flag, "");
					}
					lang = lang.replace(",", "");

					if lang.is_empty() {
						lang = "rust".to_owned();
					}

					newline(out, &indent)?;
					write!(out, "```{lang}")?;
					newline(out, &indent)
				},
				Tag::List(start) => {
					lists.push_back(start);
					Ok(())
				},
				Tag::Item => {
					indent.push_back("\t");
					match lists.back().unwrap() {
						Some(start) => write!(out, " {start}. "),
						None => write!(out, " - ")
					}
				},
				Tag::FootnoteDefinition(_) => unimplemented!(),
				Tag::Table(a) => {
					alignments = a;
					Ok(())
				},
				Tag::TableHead | Tag::TableRow => write!(out, "|"),
				Tag::TableCell => write!(out, " "),
				Tag::Emphasis => write!(out, "*"),
				Tag::Strong => write!(out, "**"),
				Tag::Strikethrough => write!(out, "~~"),
				Tag::Link(LinkType::Autolink, ..) | Tag::Link(LinkType::Email, ..) => {
					write!(out, "<")
				},
				Tag::Link(..) => write!(out, "["),
				Tag::Image(..) => write!(out, "![")
			},
			Event::End(tag) => match tag {
				Tag::Paragraph | Tag::Heading(..) => {
					newline(out, &indent)?;
					newline(out, &indent)
				},
				Tag::BlockQuote => {
					indent.pop_back();
					newline(out, &indent)
				},
				Tag::CodeBlock(_) => {
					if !has_newline {
						newline(out, &indent)?;
					}
					write!(out, "```")?;
					newline(out, &indent)?;
					newline(out, &indent)
				},
				Tag::List(_) => newline(out, &indent),
				Tag::Item => {
					indent.pop_back();
					newline(out, &indent)
				},
				Tag::FootnoteDefinition(_) => unimplemented!(),
				Tag::Table(_) => newline(out, &indent),
				Tag::TableHead => {
					newline(out, &indent)?;
					write!(out, "|")?;
					for a in &alignments {
						match a {
							Alignment::None => write!(out, " --- "),
							Alignment::Left => write!(out, ":--- "),
							Alignment::Center => write!(out, ":---:"),
							Alignment::Right => write!(out, " ---:")
						}?;
						write!(out, "|")?;
					}
					newline(out, &indent)
				},
				Tag::TableRow => newline(out, &indent),
				Tag::TableCell => write!(out, " |"),
				Tag::Emphasis => write!(out, "*"),
				Tag::Strong => write!(out, "**"),
				Tag::Strikethrough => write!(out, "~~"),
				Tag::Link(_, href, _) if href.starts_with('#') => write!(out, "]({href})"),
				Tag::Link(ty, href, name) | Tag::Image(ty, href, name) => {
					let link = format!("__link{link_idx}");
					link_idx += 1;
					match ty {
						LinkType::Inline
						| LinkType::Reference
						| LinkType::Collapsed
						| LinkType::Shortcut => {
							links.insert(link.clone(), href.to_string());
							write!(out, "][{link}]")
						},
						LinkType::ReferenceUnknown
						| LinkType::CollapsedUnknown
						| LinkType::ShortcutUnknown => {
							links.insert(link.clone(), name.to_string());
							write!(out, "][{link}]")
						},
						LinkType::Autolink | LinkType::Email => write!(out, ">")
					}
				}
			},
			Event::Text(text) => {
				has_newline = text.ends_with('\n');
				let mut first_line: bool = true;
				let mut empty: bool = true;
				for line in text.lines() {
					// if a line starts with a sharp ('#'), it has either been parsed as a header,
					// or is in a code block so should be omitted
					if line == "#"
						|| (line.starts_with('#')
							&& line.chars().nth(1).unwrap_or('a').is_whitespace())
					{
						continue;
					}

					if !first_line {
						newline(out, &indent)?;
					}
					first_line = false;

					empty = false;
					write!(out, "{line}")?;
				}
				if has_newline && !empty {
					newline(out, &indent)?;
				}
				Ok(())
			},
			Event::Code(text) => write!(out, "`{text}`"),
			Event::Html(text) => write!(out, "{text}"),
			Event::FootnoteReference(_) => unimplemented!(),
			Event::SoftBreak => write!(out, " "),
			Event::HardBreak => write!(out, "<br/>"),
			Event::Rule => write!(out, "<hr/>"),
			Event::TaskListMarker(_) => unimplemented!()
		}?;
	}

	let mut dependency_hash = DependencyHash::new(
		blake3::hash(input.rustdoc.as_bytes()),
		blake3::hash(template.as_bytes())
	);
	let mut build_link = |crate_name: &str, crate_ver: Option<&Version>, search: Option<&str>| {
		let lib_name = crate_name.replace("-", "_");
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
		dependency_hash.add_dep(crate_name.to_owned(), crate_ver.cloned(), lib_name);
		link
	};

	for link in links.keys().map(|l| l.to_owned()).collect::<Vec<_>>() {
		let mut href = links[&link].to_owned();
		if href.starts_with('`') && href.ends_with('`') {
			href = href[1..href.len() - 1].to_owned();
		}
		href = input.scope.resolve(&input.crate_name, href);
		if let Ok(path) = syn::parse_str::<Path>(&href) {
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
				links.insert(
					link,
					format!("https://doc.rust-lang.org/stable/std/?search={search}")
				);
			} else if first == "crate" {
				let (crate_name, crate_ver) = input
					.dependencies
					.get(&input.crate_name)
					.map(|(name, ver)| (name.as_str(), Some(ver)))
					.unwrap_or((&input.crate_name, None));
				links.insert(link, build_link(crate_name, crate_ver, Some(&search)));
			} else if path.segments.len() > 1 {
				let (crate_name, crate_ver) = input
					.dependencies
					.get(&first)
					.map(|(name, ver)| (name.as_str(), Some(ver)))
					.unwrap_or((&first, None));
				links.insert(link, build_link(crate_name, crate_ver, Some(&search)));
			} else if RUST_PRIMITIVES.contains(&first.as_str()) {
				links.insert(
					link,
					format!("https://doc.rust-lang.org/stable/std/primitive.{first}.html",)
				);
			} else {
				let (crate_name, crate_ver) = input
					.dependencies
					.get(&first)
					.map(|(name, ver)| (name.as_str(), Some(ver)))
					.unwrap_or((&first, None));
				links.insert(link, build_link(crate_name, crate_ver, None));
			}
		}
	}

	let mut readme_links = String::new();
	if !dependency_hash.is_empty() {
		writeln!(
			readme_links,
			" [__cargo_doc2readme_dependencies_hash]: {}",
			base64::encode_config(
				&serde_cbor::to_vec(&dependency_hash).unwrap(),
				URL_SAFE_NO_PAD
			)
		)
		.unwrap();
	}
	for (name, href) in links {
		// unwrap: writing to a String never fails
		writeln!(readme_links, " [{}]: {}", name, href).unwrap();
	}

	let mut ctx = tera::Context::new();
	ctx.insert("crate", &input.crate_name);
	if let Some(repo) = input.repository.as_deref() {
		ctx.insert("repository", &repo);
		ctx.insert(
			"repository_host",
			Url::parse(repo)?
				.host_str()
				.ok_or_else(|| anyhow!("repository url should have a host"))?
		);
	}
	if let Some(license) = input.license.as_deref() {
		ctx.insert("license", license);
	}
	ctx.insert("readme", &readme);
	ctx.insert("links", &readme_links);
	let str = Tera::one_off(template, &ctx, false /* no auto-escaping */)?;
	write!(out_file, "{}", str)?;

	Ok(())
}

#[cfg(test)]
mod tests {
	use crate::input::{InputFile, Scope};
	use cargo::core::Edition::Edition2018;
	use indoc::indoc;
	use std::collections::HashMap;

	macro_rules! test_input {
		($test_fn:ident($input:expr, $expected:expr)) => {
			#[test]
			fn $test_fn() {
				let input = InputFile {
					crate_name: "foo".to_owned(),
					repository: None,
					license: None,
					rustdoc: $input.into(),
					dependencies: HashMap::new(),
					scope: Scope::prelude(Edition2018)
				};
				println!("-- input --");
				println!("{}", input.rustdoc);
				println!("-- end input --");
				let expected: &str = $expected;
				let mut buf = Vec::<u8>::new();
				super::emit(
					input,
					"{{ readme }}{% if links != \"\" %}\n{{ links }}{% endif %}",
					&mut buf
				)
				.unwrap();
				let actual = String::from_utf8(buf).unwrap();
				pretty_assertions::assert_eq!(expected, actual);
			}
		};
	}

	test_input!(test_tag_paragraph("a\n\nb", "a\n\nb\n\n"));
	test_input!(test_tag_heading("# a", "\n## a\n\n"));
	test_input!(test_tag_blockquote("> a\n> b", "\n> a b\n> \n> \n"));

	test_input!(test_tag_codeblock_indented("\ta", "\n```rust\na\n```\n\n"));
	test_input!(test_tag_codeblock_fenced(
		"```\na\n```\n",
		"\n```rust\na\n```\n\n"
	));
	test_input!(test_tag_codeblock_fenced_lang(
		"```rust\na\n```\n",
		"\n```rust\na\n```\n\n"
	));

	test_input!(test_tag_list_ol("1. a\n3. b", " 1. a\n 1. b\n\n"));
	test_input!(test_tag_list_ol_start("3. a\n1. b", " 3. a\n 3. b\n\n"));
	test_input!(test_tag_list_ul("- a\n- b", " - a\n - b\n\n"));

	test_input!(test_tag_emphasis("*a* _b_", "*a* *b*\n\n"));
	test_input!(test_tag_strong("**a** __b__", "**a** **b**\n\n"));
	test_input!(test_tag_strikethrough("~~a~~", "~~a~~\n\n"));

	test_input!(test_tag_link_inline(
		"[a](https://example.org)",
		"[a][__link0]\n\n\n [__link0]: https://example.org\n"
	));
	test_input!(test_tag_link_reference(
		"[a][b]\n\n [b]: https://example.org",
		"[a][__link0]\n\n\n [__link0]: https://example.org\n"
	));
	test_input!(test_tag_link_reference_unknown(
		"[a][b]",
		"[a][__link0]\n\n\n [__cargo_doc2readme_dependencies_hash]: ggGBg2Fi9vY\n [__link0]: https://crates.io/crates/b\n"
	));
	test_input!(test_tag_link_reference_crate(
		"[a][crate::b]",
		"[a][__link0]\n\n\n [__cargo_doc2readme_dependencies_hash]: ggGBg2Nmb2_29g\n [__link0]: https://docs.rs/foo/latest/foo/?search=b\n"
	));
	test_input!(test_tag_link_collapsed(
		"[a][]\n\n [a]: https://example.org",
		"[a][__link0]\n\n\n [__link0]: https://example.org\n"
	));
	test_input!(test_tag_link_collapsed_unknown(
		"[a][]",
		"[a][__link0]\n\n\n [__cargo_doc2readme_dependencies_hash]: ggGBg2Fh9vY\n [__link0]: https://crates.io/crates/a\n"
	));
	test_input!(test_tag_link_shortcut(
		"[a]\n\n [a]: https://example.org",
		"[a][__link0]\n\n\n [__link0]: https://example.org\n"
	));
	test_input!(test_tag_link_shortcut_unknown(
		"[a]",
		"[a][__link0]\n\n\n [__cargo_doc2readme_dependencies_hash]: ggGBg2Fh9vY\n [__link0]: https://crates.io/crates/a\n"
	));
	test_input!(test_tag_link_autolink(
		"<https://example.org>",
		"<https://example.org>\n\n"
	));
	test_input!(test_tag_link_email(
		"<noreply@example.org>",
		"<noreply@example.org>\n\n"
	));
	test_input!(test_local_link("[a](#a)", "[a](#a)\n\n"));

	#[rustfmt::skip]
	const TABLE: &str = indoc!(r#"
		| a | b | c | d |
		| --- |:--- |:---:| ---:|
		| 1 | 2 | 3 | 4 |
	"#);
	test_input!(test_table(TABLE, &format!("{}\n", TABLE)));

	#[rustfmt::skip]
	const NESTED: &str = indoc!(r#"
		# Nested Madness
		
		An example with lots of nesting
		
		 1. First, you think
			
			
		 1. Then, you write down:
			
			
			```rust
			fn main() {
				println!("Hello World");
			}
			```
			
			 3. You realize that your idea is too nested
				> like this table you saw recently
				> 
				> | nested code |
				> |:---:|
				> | is bad |
				> 
				>  - there you have it
				> 
				> 
				
			
			
		
	"#);
	test_input!(test_nested(NESTED, &format!("\n#{}", NESTED)));
}
