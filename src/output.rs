use crate::input::InputFile;
use once_cell::sync::Lazy;
use pulldown_cmark::{Alignment, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag};
use regex::Regex;
use std::{
	collections::{BTreeMap, VecDeque},
	io::{self, Write}
};

const DEFAULT_CODEBLOCK_LANG: &str = "rust";
const RUST_PRIMITIVES: &[&str] = &[
	// https://doc.rust-lang.org/stable/std/primitive/index.html#reexports
	"bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "str", "u128", "u16", "u32", "u64", "u8",
	"usize"
];

fn broken_link_callback<'a>(lnk: BrokenLink<'_>) -> Option<(CowStr<'a>, CowStr<'a>)> {
	Some(("".into(), lnk.reference.to_string().into()))
}

fn newline(out: &mut dyn Write, indent: &VecDeque<&'static str>) -> io::Result<()> {
	write!(out, "\n")?;
	for s in indent {
		write!(out, "{}", s)?;
	}
	Ok(())
}

pub fn emit(input: InputFile, out: &mut dyn Write) -> anyhow::Result<()> {
	// we need this broken link callback for the purpose of broken links being parsed as links
	let mut broken_link_callback = broken_link_callback;
	let parser = Parser::new_with_broken_link_callback(&input.rustdoc, Options::all(), Some(&mut broken_link_callback));

	let mut alignments: Vec<Alignment> = Vec::new();
	let mut has_newline = true;
	let mut links: BTreeMap<String, String> = BTreeMap::new();
	let mut link_idx: u32 = 0;
	let mut indent: VecDeque<&'static str> = VecDeque::new();
	let mut lists: VecDeque<Option<u64>> = VecDeque::new();

	for ev in parser {
		//println!("[DEBUG] ev = {:?}", ev);
		match ev {
			Event::Start(tag) => match tag {
				Tag::Paragraph => Ok(()),
				Tag::Heading(lvl) => {
					newline(out, &indent)?;
					for _ in 0..=lvl {
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
					let lang: &str = &lang;
					let lang = match lang {
						"" => DEFAULT_CODEBLOCK_LANG,
						lang if lang.starts_with("rust,") => "rust",
						lang => lang
					};
					newline(out, &indent)?;
					write!(out, "```{}", lang)?;
					newline(out, &indent)
				},
				Tag::List(start) => {
					lists.push_back(start);
					Ok(())
				},
				Tag::Item => {
					indent.push_back("\t");
					match lists.back().unwrap() {
						Some(start) => write!(out, " {}. ", start),
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
				Tag::Link(LinkType::Autolink, ..) | Tag::Link(LinkType::Email, ..) => write!(out, "<"),
				Tag::Link(..) => write!(out, "["),
				Tag::Image(..) => write!(out, "![")
			},
			Event::End(tag) => match tag {
				Tag::Paragraph | Tag::Heading(_) => {
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
					newline(out, &indent)
				},
				Tag::List(_) => Ok(()),
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
				Tag::Link(_, href, _) if href.starts_with("#") => write!(out, "]({})", href),
				Tag::Link(ty, href, name) | Tag::Image(ty, href, name) => {
					let link = format!("__link{}", link_idx);
					link_idx += 1;
					match ty {
						LinkType::Inline | LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut => {
							links.insert(link.clone(), href.to_string());
							write!(out, "][{}]", link)
						},
						LinkType::ReferenceUnknown | LinkType::CollapsedUnknown | LinkType::ShortcutUnknown => {
							links.insert(link.clone(), name.to_string());
							write!(out, "][{}]", link)
						},
						LinkType::Autolink | LinkType::Email => write!(out, ">")
					}
				}
			},
			Event::Text(text) => {
				has_newline = text.ends_with("\n");
				let mut first_line: bool = true;
				for line in text.lines() {
					// if a line starts with a sharp ('#'), it has either been parsed as a header,
					// or is in a code block so should be omitted
					if line == "#" || (line.starts_with('#') && line.chars().nth(1).unwrap_or('a').is_whitespace()) {
						continue;
					}

					if !first_line {
						newline(out, &indent)?;
					}
					first_line = false;

					write!(out, "{}", line)?;
				}
				if has_newline {
					newline(out, &indent)?;
				}
				Ok(())
			},
			Event::Code(text) => write!(out, "`{}`", text),
			Event::Html(text) => write!(out, "{}", text),
			Event::FootnoteReference(_) => unimplemented!(),
			Event::SoftBreak => write!(out, " "),
			Event::HardBreak => write!(out, "<br/>"),
			Event::Rule => write!(out, "<hr/>"),
			Event::TaskListMarker(_) => unimplemented!()
		}?;
	}

	// https://regex101.com/r/SzD4j1/1
	static RUST_LINK_REGEX: Lazy<Regex> = Lazy::new(|| {
		Regex::new("^(((::)?(?P<first>[a-zA-Z_][a-zA-Z0-9_]*))(?P<segments>(::[a-zA-Z_][a-zA-Z0-9_]*)*)::)?(?P<name>[a-zA-Z_][a-zA-Z0-9_]*)$").unwrap()
	});
	for link in links.keys().map(|l| l.to_owned()).collect::<Vec<_>>() {
		let mut href = links[&link].to_owned();
		if href.starts_with("`") && href.ends_with("`") {
			href = href[1..href.len() - 1].to_owned();
		}
		loop {
			if let Some(c) = RUST_LINK_REGEX.captures(&href) {
				let first = c.name("first").map(|g| g.as_str()).unwrap_or_default();
				let segments = c.name("segments").map(|g| g.as_str()).unwrap_or_default();
				let name = c.name("name").map(|g| g.as_str()).unwrap_or_default();
				println!("[DEBUG] {:?} => {:?} {:?} {:?}", href, first, segments, name);

				// TODO more sophisticated link generation
				if first == "std" || first == "alloc" || first == "core" {
					links.insert(
						link,
						format!(
							"https://doc.rust-lang.org/stable/std/?search={crate}{segments}::{name}",
							crate = first,
							segments = segments,
							name = name
						)
					);
				} else if !first.is_empty() {
					let (crate_name, crate_ver) = input
						.dependencies
						.get(first)
						.map(|(name, ver)| (name.as_str(), ver.to_string()))
						.unwrap_or((first, "*".to_string()));
					links.insert(
						link,
						format!(
							"https://docs.rs/{crate}/{ver}/{crate}/?search={crate}{segments}::{name}",
							crate = crate_name,
							ver = crate_ver,
							segments = segments,
							name = name
						)
					);
				} else if input.scope.uses.contains_key(name) {
					href = input.scope.uses[name].clone();
					continue;
				} else if RUST_PRIMITIVES.contains(&name) {
					links.insert(link, format!("https://doc.rust-lang.org/stable/std/primitive.{}.html", name));
				} else {
					let (crate_name, crate_ver) = input
						.dependencies
						.get(name)
						.map(|(name, ver)| (name.as_str(), format!("/{}", ver)))
						.unwrap_or((name, String::new()));
					links.insert(link, format!("https://crates.io/crates/{}{}", crate_name, crate_ver));
				}
			}
			break;
		}
	}

	if !links.is_empty() {
		write!(out, "\n")?;
		for (name, href) in links {
			write!(out, " [{}]: {}\n", name, href)?;
		}
	}

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
					rustdoc: $input.into(),
					dependencies: HashMap::new(),
					scope: Scope::prelude(Edition2018)
				};
				println!("-- input --");
				println!("{}", input.rustdoc);
				println!("-- end input --");
				let expected: &str = $expected;
				let mut buf = Vec::<u8>::new();
				super::emit(input, &mut buf).unwrap();
				let actual = String::from_utf8(buf).unwrap();
				pretty_assertions::assert_eq!(expected, actual);
			}
		};
	}

	test_input!(test_tag_paragraph("a\n\nb", "a\n\nb\n\n"));
	test_input!(test_tag_heading("# a", "\n## a\n\n"));
	test_input!(test_tag_blockquote("> a\n> b", "\n> a b\n> \n> \n"));

	test_input!(test_tag_codeblock_indented("\ta", "\n```\na\n```\n"));
	test_input!(test_tag_codeblock_fenced("```\na\n```\n", "\n```\na\n```\n"));
	test_input!(test_tag_codeblock_fenced_lang("```rust\na\n```\n", "\n```rust\na\n```\n"));

	test_input!(test_tag_list_ol("1. a\n3. b", " 1. a\n 1. b\n"));
	test_input!(test_tag_list_ol_start("3. a\n1. b", " 3. a\n 3. b\n"));
	test_input!(test_tag_list_ul("- a\n- b", " - a\n - b\n"));

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
	test_input!(test_tag_link_reference_unknown("[a][b]", "[a][__link0]\n\n\n [__link0]: b\n"));
	test_input!(test_tag_link_collapsed(
		"[a][]\n\n [a]: https://example.org",
		"[a][__link0]\n\n\n [__link0]: https://example.org\n"
	));
	test_input!(test_tag_link_collapsed_unknown("[a][]", "[a][__link0]\n\n\n [__link0]: a\n"));
	test_input!(test_tag_link_shortcut(
		"[a]\n\n [a]: https://example.org",
		"[a][__link0]\n\n\n [__link0]: https://example.org\n"
	));
	test_input!(test_tag_link_shortcut_unknown("[a]", "[a][__link0]\n\n\n [__link0]: a\n"));
	test_input!(test_tag_link_autolink("<https://example.org>", "<https://example.org>\n\n"));
	test_input!(test_tag_link_email("<noreply@example.org>", "<noreply@example.org>\n\n"));
	test_input!(test_local_link("[a](#a)", "[a](#a)\n\n"));

	#[cfg_attr(rustfmt, rustfmt_skip)]
	const TABLE: &str = indoc!(r#"
		| a | b | c | d |
		| --- |:--- |:---:| ---:|
		| 1 | 2 | 3 | 4 |
	"#);
	test_input!(test_table(TABLE, &format!("{}\n", TABLE)));

	#[cfg_attr(rustfmt, rustfmt_skip)]
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
				
			
	"#);
	test_input!(test_nested(NESTED, &format!("\n#{}", NESTED)));
}
