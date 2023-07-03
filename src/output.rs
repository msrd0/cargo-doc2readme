use crate::{
	input::{InputFile, Scope, TargetType},
	links::Links
};
use log::debug;
use pulldown_cmark::{
	Alignment, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag
};
use semver::VersionReq;
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

pub struct ResolvedLink {
	pub path: String,
	pub link_type: Option<crate::input::LinkType>
}

impl Scope {
	pub fn resolve(&self, crate_name: &str, path: String) -> ResolvedLink {
		self.resolve_impl(crate_name, None, path)
	}

	pub fn resolve_impl(
		&self,
		crate_name: &str,
		link_type: Option<crate::input::LinkType>,
		path: String
	) -> ResolvedLink {
		if !path.starts_with("::") {
			// split path into segments, ignoring <...> generics
			let mut path = path.clone();
			loop {
				let idx = match (path.find('<'), path.rfind('>')) {
					(Some(idx1), Some(idx2)) if idx1 < idx2 => idx1,
					_ => break
				};
				let mut end = idx + 1;
				let mut depth: usize = 1;
				for ch in path[end ..].chars() {
					if ch == '<' {
						depth += 1;
					} else if ch == '>' {
						depth -= 1;
					}
					end += ch.len_utf8();

					if depth == 0 {
						break;
					}
				}
				path.replace_range(idx .. end, "");
			}
			debug!("Resolving path {path:?}");
			let mut segments = path.split("::").collect::<Vec<_>>();
			if segments[0] == "crate" {
				segments[0] = crate_name;
			}

			// check if we can resolve anything
			if self.scope.contains_key(segments[0]) {
				let paths = &self.scope[segments[0]];
				if let Some((path_link_type, path)) = paths.front() {
					segments[0] = path;
					let path = segments.join("::");
					if path.starts_with("::") {
						return ResolvedLink {
							path,
							link_type: if segments.len() == 1 {
								Some(*path_link_type)
							} else {
								link_type
							}
						};
					}
					return self.resolve(crate_name, segments.join("::"));
				}
			}
		}

		ResolvedLink { path, link_type }
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
			let href = self.input.scope.resolve(&self.input.crate_name, href);

			if let Ok(path) = syn::parse_str::<Path>(&href.path) {
				self.links
					.insert(link, links.build_link(&path, href.link_type, self.input));
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
	#[serde(rename = "crate_version")]
	krate_version: &'a str,
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
		krate_version: &input.crate_version,
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
