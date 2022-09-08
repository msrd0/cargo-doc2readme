use itertools::Itertools as _;
use log::debug;
use std::{io, iter::Peekable};

enum Attr {
	Doc {
		/// The indent before the doc comment
		indent: String,
		/// The style (e.g. `//!`) of the doc comment
		style: String,
		/// The indent of the comment itself
		comment_indent: String,
		/// The comment trimmed of any indent
		comment: String
	},
	Verbatim {
		/// An entire line of source code
		line: String
	}
}

pub struct Preprocessor<L>
where
	L: Iterator<Item = io::Result<String>>
{
	/// Remaining lines to read from the underlying reader
	lines: Peekable<L>,
	/// Buffer of processed lines ready to be read
	buf: Vec<u8>
}

impl<R> Preprocessor<io::Lines<R>>
where
	R: io::BufRead
{
	// https://github.com/rust-lang/rust/issues/101582
	#[allow(dead_code)]
	pub fn new(read: R) -> Self {
		Self {
			lines: read.lines().peekable(),
			buf: Vec::new()
		}
	}
}

fn take3<I>(iter: &mut I) -> Option<[I::Item; 3]>
where
	I: Iterator
{
	Some([iter.next()?, iter.next()?, iter.next()?])
}

impl<L> Preprocessor<L>
where
	L: Iterator<Item = io::Result<String>>
{
	fn fill_buf(&mut self) -> io::Result<()> {
		let mut attrs = Vec::<Attr>::new();
		while let Some(line) = self.lines.peek() {
			let line = match line {
				Ok(line) => line,
				Err(_) => break
			};
			let trimmed = line.trim_start();
			if trimmed.starts_with("//!") || trimmed.starts_with("///") {
				let line = self.lines.next().unwrap().unwrap();
				let mut chars = line.chars();
				let indent = chars
					.peeking_take_while(|ch| ch.is_whitespace())
					.collect::<String>();
				let style = take3(&mut chars).unwrap().into_iter().collect::<String>();
				let comment_indent = chars
					.peeking_take_while(|ch| ch.is_whitespace())
					.collect::<String>();
				let comment = chars.collect::<String>();
				attrs.push(Attr::Doc {
					indent,
					style,
					comment_indent,
					comment
				});
			} else if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
				// line that might sit between doc comments
				attrs.push(Attr::Verbatim {
					line: self.lines.next().unwrap().unwrap()
				});
			} else {
				// we've encountered the end of the doc comment
				break;
			}
		}

		let mut common_indent: Option<String> = None;
		for attr in &attrs {
			if let Attr::Doc {
				comment_indent,
				comment,
				..
			} = attr
			{
				match &common_indent {
					Some(common) if !comment_indent.starts_with(common) && !comment.is_empty() => {
						common_indent = Some(
							common
								.chars()
								.zip(comment_indent.chars())
								.take_while(|(lhs, rhs)| lhs == rhs)
								.map(|(ch, _)| ch)
								.collect()
						);
					},
					None => {
						common_indent = Some(
							comment_indent
								.chars()
								.take_while(|ch| ch.is_whitespace())
								.collect()
						);
					},
					_ => {}
				}
			}
		}
		let common_indent_len = common_indent
			.map(|common| common.as_bytes().len())
			.unwrap_or(0);
		debug!(
			"Removing common indent of {common_indent_len} bytes from {} lines",
			attrs.len()
		);

		for attr in attrs {
			match attr {
				Attr::Doc {
					indent,
					style,
					comment_indent,
					comment
				} => {
					self.buf.extend_from_slice(indent.as_bytes());
					self.buf.extend_from_slice(style.as_bytes());
					self.buf
						.extend(comment_indent.bytes().skip(common_indent_len));
					self.buf.extend_from_slice(comment.as_bytes());
				},
				Attr::Verbatim { line } => {
					self.buf.extend_from_slice(line.as_bytes());
				}
			}
			self.buf.push(b'\n');
		}

		// the next line should not be part of the doc comment
		if let Some(line) = self.lines.next() {
			self.buf.extend_from_slice(line?.as_bytes());
			self.buf.push(b'\n');
		}

		Ok(())
	}
}

impl<L> io::Read for Preprocessor<L>
where
	L: Iterator<Item = io::Result<String>>
{
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		if self.buf.is_empty() {
			self.fill_buf()?;
		}

		let bytes = buf.len().min(self.buf.len());
		let (head, tail) = self.buf.split_at(bytes);
		buf[0..bytes].clone_from_slice(head);
		self.buf = tail.to_owned();
		Ok(bytes)
	}
}
