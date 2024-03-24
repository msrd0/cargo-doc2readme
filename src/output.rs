use crate::{
	input::{InputFile, Scope, TargetType},
	links::Links
};
use itertools::Itertools as _;
use log::debug;
use pulldown_cmark::{
	BrokenLink, CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Options, Parser,
	Tag, TagEnd
};
use semver::Version;
use serde::Serialize;
use std::{
	collections::BTreeMap,
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

fn is_hidden_codeblock_line(line: &str) -> bool {
	line == "#"
		|| (line.starts_with('#') && line.chars().nth(1).unwrap_or('a').is_whitespace())
}

struct EventFilter<'a, I: Iterator<Item = Event<'a>>> {
	iter: I,
	links: &'a mut BTreeMap<String, String>,

	in_code_block: bool,
	link_idx: usize
}

impl<'a, I: Iterator<Item = Event<'a>>> EventFilter<'a, I> {
	fn new(iter: I, links: &'a mut BTreeMap<String, String>) -> Self {
		Self {
			iter,
			links,

			in_code_block: false,
			link_idx: 0
		}
	}
}

impl<'a, I: Iterator<Item = Event<'a>>> Iterator for EventFilter<'a, I> {
	type Item = Event<'a>;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			break Some(match self.iter.next()? {
				Event::Start(tag) => Event::Start(match tag {
					// we increase headings by 1 level
					Tag::Heading {
						level,
						id,
						classes,
						attrs
					} => {
						let level = match level {
							HeadingLevel::H1 => HeadingLevel::H2,
							HeadingLevel::H2 => HeadingLevel::H3,
							HeadingLevel::H3 => HeadingLevel::H4,
							HeadingLevel::H4 => HeadingLevel::H5,
							_ => HeadingLevel::H6
						};
						Tag::Heading {
							level,
							id,
							classes,
							attrs
						}
					},

					// we record codeblocks and adjust their language
					Tag::CodeBlock(kind) => {
						debug_assert!(
							!self.in_code_block,
							"Recursive codeblocks, wtf???"
						);
						self.in_code_block = true;
						Tag::CodeBlock(CodeBlockKind::Fenced(match kind {
							CodeBlockKind::Indented => DEFAULT_CODEBLOCK_LANG.into(),
							CodeBlockKind::Fenced(lang) => {
								let mut lang: String = (*lang).to_owned();
								for flag in RUSTDOC_CODEBLOCK_FLAGS {
									lang = lang.replace(flag, "");
								}
								let mut lang: CowStr<'_> = lang.replace(',', "").into();
								if lang.is_empty() {
									lang = DEFAULT_CODEBLOCK_LANG.into();
								}
								lang
							}
						}))
					},

					Tag::Link {
						link_type,
						dest_url,
						title,
						id
					} if dest_url.starts_with('#')
						|| link_type == LinkType::Autolink
						|| link_type == LinkType::Email =>
					{
						Tag::Link {
							link_type,
							dest_url,
							title,
							id
						}
					},
					Tag::Link {
						dest_url,
						title,
						id,
						link_type
					} => {
						eprintln!(
							"Link: dest_url={dest_url:?}, title={title:?}, id={id:?}"
						);
						let link = format!("__link{}", self.link_idx);
						self.link_idx += 1;
						if !dest_url.is_empty() {
							self.links.insert(link.clone(), dest_url.to_string());
						} else if !id.is_empty() {
							self.links.insert(link.clone(), id.to_string());
						} else if !title.is_empty() {
							self.links.insert(link.clone(), title.to_string());
						} else {
							break Some(Event::Start(Tag::Link {
								link_type,
								dest_url,
								title,
								id
							}));
						}
						Tag::Link {
							// pulldown-cmark-to-cmark does not support outputting
							// unresolved reference-style links so we have to do
							// it this stupid way
							link_type: LinkType::Inline,
							dest_url: link.into(),
							title: "".into(),
							id
						}
					},

					// we don't need to modify any other tags
					tag => tag
				}),

				Event::End(tag) => Event::End(match tag {
					// we record when a codeblock ends
					TagEnd::CodeBlock => {
						debug_assert!(
							self.in_code_block,
							"Ending non-started code block, wtf???"
						);
						self.in_code_block = false;
						TagEnd::CodeBlock
					},
					// we don't need to modify any other tags
					tag => tag
				}),

				Event::Text(text) if self.in_code_block => {
					let filtered = text
						.lines()
						.filter(|line| !is_hidden_codeblock_line(line))
						.join("\n");
					if filtered.is_empty() {
						continue;
					}
					Event::Text(filtered.into())
				},

				ev => ev
			});
		}
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

		let options = pulldown_cmark_to_cmark::Options {
			code_block_token_count: 3,
			..Default::default()
		};
		pulldown_cmark_to_cmark::cmark_with_options(
			EventFilter::new(parser.into_iter(), &mut self.links),
			&mut self.readme,
			options
		)?;

		// we need to replace the links generated by pulldown-cmark-to-cmark with
		// reference-style links
		let mut i = 0;
		while i < self.readme.len() {
			let Some(idx) = self.readme[i ..].find("(__link") else {
				break;
			};
			let idx = idx + i;
			let Some(idx2) = self.readme[idx ..].find(')') else {
				break;
			};
			let idx2 = idx2 + idx;
			i = idx2;

			self.readme.replace_range(idx ..= idx, "[");
			self.readme.replace_range(idx2 ..= idx2, "]");
		}

		if !self.readme.ends_with('\n') {
			self.readme.push('\n');
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
	rust_version: Option<&'a Version>,

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
		krate_version: &format!("{}", input.crate_version),
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
