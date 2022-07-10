use crate::{
	depinfo::DependencyInfo,
	input::{InputFile, Scope}
};
use anyhow::anyhow;
use either::Either;
use itertools::Itertools;
use pulldown_cmark::{
	Alignment, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag
};
use semver::Version;
use std::{
	collections::{BTreeMap, VecDeque},
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
					lang = lang.replace(',', "");

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

	const MARKDOWN_VERSION: u8 = 0;
	let mut dependency_info = DependencyInfo::new(MARKDOWN_VERSION, template, &input.rustdoc);
	let mut build_link = |crate_name: &str, crate_ver: Option<&Version>, search: Option<&str>| {
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
		dependency_info.add_dependency(crate_name.to_owned(), crate_ver.cloned(), lib_name);
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
	if !dependency_info.is_empty() {
		writeln!(
			readme_links,
			" [__cargo_doc2readme_dependencies_info]: {}",
			dependency_info.encode()
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
