use crate::{
	depinfo::DependencyInfo,
	input::{Dependency, InputFile, Scope, TargetType}
};
use either::Either;
use itertools::Itertools;
use pulldown_cmark::{
	Alignment, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag
};
use semver::{Version, VersionReq};
use serde::Serialize;
use std::{
	collections::{BTreeMap, VecDeque},
	fmt::{self, Write as _},
	io
};
use syn::Path;
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
	"bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "str",
	"u128", "u16", "u32", "u64", "u8", "usize"
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
			let paths = &self.scope[segments[0]];
			if let Some((_, path)) = paths.front() {
				segments[0] = path;
				return self.resolve(crate_name, segments.join("::"));
			}
		}
		path
	}
}

fn broken_link_callback<'a>(lnk: BrokenLink<'_>) -> Option<(CowStr<'a>, CowStr<'a>)> {
	Some(("".into(), lnk.reference.to_string().into()))
}

fn newline(
	out: &mut dyn fmt::Write,
	indent: &VecDeque<&'static str>,
	has_newline: &mut bool
) -> fmt::Result {
	writeln!(out)?;
	for s in indent {
		write!(out, "{s}")?;
	}
	*has_newline = true;
	Ok(())
}

struct Links {
	deps: DependencyInfo
}

impl Links {
	fn new(template: &str, rustdoc: &str) -> Self {
		Self {
			deps: DependencyInfo::new(template, rustdoc)
		}
	}

	fn build_link(
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

struct Readme<'a> {
	template: &'a str,
	input: &'a InputFile,

	/// Holds the main markdown part of the readme that was created from the rustdoc,
	/// but does not include any parts of the template or the links.
	readme: String,

	/// Holds the link part of the markdown.
	readme_links: String,

	links: BTreeMap<String, String>
}

impl<'a> Readme<'a> {
	fn new(template: &'a str, input: &'a InputFile) -> Self {
		Self {
			template,
			input,
			readme: String::new(),
			readme_links: String::new(),
			links: BTreeMap::new()
		}
	}

	fn write_markdown(&mut self) -> fmt::Result {
		// we need this broken link callback for the purpose of broken links being parsed as links
		let mut broken_link_callback = broken_link_callback;
		let parser = Parser::new_with_broken_link_callback(
			&self.input.rustdoc,
			Options::all(),
			Some(&mut broken_link_callback)
		);

		let mut alignments: Vec<Alignment> = Vec::new();
		let mut has_newline = true;
		let mut link_idx: u32 = 0;
		let mut indent: VecDeque<&'static str> = VecDeque::new();
		let mut lists: VecDeque<Option<u64>> = VecDeque::new();

		let out = &mut self.readme;
		for ev in parser {
			match ev {
				Event::Start(tag) => match tag {
					Tag::Paragraph => Ok(()),
					Tag::Heading(lvl, ..) => {
						newline(out, &indent, &mut has_newline)?;
						for _ in 0 ..= lvl as u8 {
							write!(out, "#")?;
						}
						write!(out, " ")
					},
					Tag::BlockQuote => {
						newline(out, &indent, &mut has_newline)?;
						indent.push_back("> ");
						write!(out, "> ")
					},
					Tag::CodeBlock(CodeBlockKind::Indented) => {
						newline(out, &indent, &mut has_newline)?;
						write!(out, "```{}", DEFAULT_CODEBLOCK_LANG)?;
						newline(out, &indent, &mut has_newline)
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

						newline(out, &indent, &mut has_newline)?;
						write!(out, "```{lang}")?;
						newline(out, &indent, &mut has_newline)
					},
					Tag::List(start) => {
						lists.push_back(start);
						Ok(())
					},
					Tag::Item => {
						if !has_newline {
							newline(out, &indent, &mut has_newline)?;
						}
						indent.push_back("\t");
						match lists.back_mut().unwrap() {
							Some(start) => {
								write!(out, " {start}. ")?;
								*start += 1;
								Ok(())
							},
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
					Tag::Link(LinkType::Autolink, ..)
					| Tag::Link(LinkType::Email, ..) => {
						write!(out, "<")
					},
					Tag::Link(..) => write!(out, "["),
					Tag::Image(..) => write!(out, "![")
				},
				Event::End(tag) => match tag {
					Tag::Paragraph | Tag::Heading(..) => {
						newline(out, &indent, &mut has_newline)?;
						newline(out, &indent, &mut has_newline)
					},
					Tag::BlockQuote => {
						indent.pop_back();
						newline(out, &indent, &mut has_newline)
					},
					Tag::CodeBlock(_) => {
						if !has_newline {
							newline(out, &indent, &mut has_newline)?;
						}
						write!(out, "```")?;
						newline(out, &indent, &mut has_newline)?;
						newline(out, &indent, &mut has_newline)
					},
					Tag::List(_) => {
						let pop = lists.pop_back();
						debug_assert!(pop.is_some());
						newline(out, &indent, &mut has_newline)
					},
					Tag::Item => {
						indent.pop_back();
						newline(out, &indent, &mut has_newline)
					},
					Tag::FootnoteDefinition(_) => unimplemented!(),
					Tag::Table(_) => newline(out, &indent, &mut has_newline),
					Tag::TableHead => {
						newline(out, &indent, &mut has_newline)?;
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
						newline(out, &indent, &mut has_newline)
					},
					Tag::TableRow => newline(out, &indent, &mut has_newline),
					Tag::TableCell => write!(out, " |"),
					Tag::Emphasis => write!(out, "*"),
					Tag::Strong => write!(out, "**"),
					Tag::Strikethrough => write!(out, "~~"),
					Tag::Link(_, href, _) if href.starts_with('#') => {
						write!(out, "]({href})")
					},
					Tag::Link(ty, href, name) | Tag::Image(ty, href, name) => {
						let link = format!("__link{link_idx}");
						link_idx += 1;
						match ty {
							LinkType::Inline
							| LinkType::Reference
							| LinkType::Collapsed
							| LinkType::Shortcut => {
								self.links.insert(link.clone(), href.to_string());
								write!(out, "][{link}]")
							},
							LinkType::ReferenceUnknown
							| LinkType::CollapsedUnknown
							| LinkType::ShortcutUnknown => {
								self.links.insert(link.clone(), name.to_string());
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
							newline(out, &indent, &mut has_newline)?;
						}
						first_line = false;

						empty = false;
						write!(out, "{line}")?;
					}
					if has_newline && !empty {
						newline(out, &indent, &mut has_newline)?;
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

		Ok(())
	}

	fn write_links(&mut self) {
		let mut links = Links::new(self.template, &self.input.rustdoc);
		for link in self.links.keys().map(|l| l.to_owned()).collect::<Vec<_>>() {
			let mut href = self.links[&link].to_owned();
			if href.starts_with('`') && href.ends_with('`') {
				href = href[1 .. href.len() - 1].to_owned();
			}
			href = self.input.scope.resolve(&self.input.crate_name, href);
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
					self.links.insert(link, links.std_link(search.as_str()));
				} else if first == "crate" {
					let (crate_name, crate_ver) = self
						.input
						.dependencies
						.get(&self.input.crate_name)
						.map(Dependency::as_tuple)
						.unwrap_or((&self.input.crate_name, None));
					self.links.insert(
						link,
						links.build_link(crate_name, crate_ver, Some(&search))
					);
				} else if path.segments.len() > 1 {
					let (crate_name, crate_ver) = self
						.input
						.dependencies
						.get(&first)
						.map(Dependency::as_tuple)
						.unwrap_or((&first, None));
					self.links.insert(
						link,
						links.build_link(crate_name, crate_ver, Some(&search))
					);
				} else if RUST_PRIMITIVES.contains(&first.as_str()) {
					self.links
						.insert(link, links.primitive_link(first.as_str()));
				} else {
					let (crate_name, crate_ver) = self
						.input
						.dependencies
						.get(&first)
						.map(Dependency::as_tuple)
						.unwrap_or((&first, None));
					self.links
						.insert(link, links.build_link(crate_name, crate_ver, None));
				}
			}
		}

		if !links.deps.is_empty() {
			writeln!(
				self.readme_links,
				" [__cargo_doc2readme_dependencies_info]: {}",
				links.deps.encode()
			)
			.unwrap();
		}
		for (name, href) in &self.links {
			// unwrap: writing to a String never fails
			writeln!(self.readme_links, " [{}]: {}", name, href).unwrap();
		}
	}
}

#[derive(Serialize)]
struct TemplateContext<'a> {
	#[serde(rename = "crate")]
	krate: &'a str,
	target: TargetType,

	repository: Option<&'a str>,
	repository_host: Option<String>,

	license: Option<&'a str>,
	rust_version: Option<&'a VersionReq>,

	readme: String,
	links: String
}

pub fn emit(
	input: InputFile,
	template: &str,
	out_file: &mut dyn io::Write
) -> anyhow::Result<()> {
	let mut readme = Readme::new(template, &input);

	// unwrap: This will never fail since we're only writing to a String.
	// it is just inconvenient to write .unwrap() behind every single write!() invocation
	readme.write_markdown().unwrap();

	readme.write_links();

	let repository = input.repository.as_deref();
	let ctx = TemplateContext {
		krate: &input.crate_name,
		target: input.target_type,
		repository,
		repository_host: repository.and_then(|repo| {
			let url = Url::parse(repo).ok();
			url.as_ref()
				.and_then(|url| url.host_str())
				.map(String::from)
		}),
		license: input.license.as_deref(),
		rust_version: input.rust_version.as_ref(),
		readme: readme.readme,
		links: readme.readme_links
	};

	let mut env = minijinja::Environment::new();
	env.add_template("template", template)?;
	env.get_template("template")?
		.render_to_write(ctx, out_file)?;

	Ok(())
}
