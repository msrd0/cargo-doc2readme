use ariadne::{Color, Label, Report, ReportKind};
use std::{io, ops::Range};

pub type Span = Range<usize>;

pub struct Diagnostic {
	filename: String,
	code: String,
	reports: Vec<Report<(String, Span)>>,
	fail: bool
}

impl Diagnostic {
	pub fn new(filename: String, code: String) -> Self {
		Self {
			filename,
			code,
			reports: Vec::new(),
			fail: false
		}
	}

	pub fn is_fail(&self) -> bool {
		self.fail
	}

	pub fn print(&self) -> io::Result<()> {
		self.print_to(io::stderr())
	}

	pub fn print_to<W: io::Write>(&self, mut w: W) -> io::Result<()> {
		let mut cache = (self.filename.clone(), self.code.clone().into());
		for r in &self.reports {
			r.write(&mut cache, &mut w)?;
		}
		Ok(())
	}

	fn offset(&self, at: proc_macro2::LineColumn) -> usize {
		let line_offset: usize = self
			.code
			.split('\n')
			.take(at.line - 1)
			.map(|line| line.chars().count() + 1)
			.sum();
		line_offset + at.column
	}

	/// Info without a code label.
	pub fn info<T>(&mut self, msg: T)
	where
		T: ToString
	{
		self.reports.push(
			Report::build(
				ReportKind::Custom("info", Color::Green),
				self.filename.clone(),
				0
			)
			.with_message(msg)
			.finish()
		);
	}

	/// Warning without a code label.
	pub fn warn<T>(&mut self, msg: T)
	where
		T: ToString
	{
		self.reports.push(
			Report::build(ReportKind::Warning, self.filename.clone(), 0)
				.with_message(msg)
				.finish()
		);
	}

	/// Warning with a code label.
	pub fn warn_with_label<T, L>(&mut self, msg: T, span: proc_macro2::Span, label: L)
	where
		T: ToString,
		L: ToString
	{
		let span = self.offset(span.start()) .. self.offset(span.end());
		self.reports.push(
			Report::build(ReportKind::Warning, self.filename.clone(), span.start)
				.with_message(msg)
				.with_label(Label::new((self.filename.clone(), span)).with_message(label))
				.finish()
		);
	}

	/// Syntax error with the code span from syn's error.
	pub fn syntax_error(&mut self, err: syn::Error) {
		let mut report = Report::build(
			ReportKind::Error,
			self.filename.clone(),
			self.offset(err.span().start())
		);
		report.set_message("Syntax Error");
		for err in err {
			let span = err.span();
			report.add_label(
				Label::new((
					self.filename.clone(),
					self.offset(span.start()) .. self.offset(span.end())
				))
				.with_message(err)
			);
		}
		self.reports.push(report.finish());
		self.fail = true;
	}

	/// Error without a code label.
	pub fn error<T>(&mut self, msg: T)
	where
		T: ToString
	{
		self.reports.push(
			Report::build(ReportKind::Error, self.filename.clone(), 0)
				.with_message(msg)
				.finish()
		);
		self.fail = true;
	}
}
