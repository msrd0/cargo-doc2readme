//! `cargo doc2readme` is a cargo subcommand to create a readme file to display on [GitHub] or [crates.io],
//! containing the rustdoc comments from your code.
//!
//!  [crates.io]: https://crates.io
//!  [GitHub]: https://github.com

use clap::Clap;
use std::{env, fs::File};

mod input;
mod output;

#[derive(Clap)]
enum Subcommand {
	Doc2readme(Args)
}

#[derive(Clap)]
struct Args {
	/// Output File.
	#[clap(short, long)]
	out: String
}

#[derive(Clap)]
struct CmdLine {
	#[clap(subcommand)]
	cmd: Subcommand
}

fn main() {
	let args = match env::args().nth(1) {
		Some(subcmd) if subcmd == "doc2readme" => match CmdLine::parse().cmd {
			Subcommand::Doc2readme(args) => args
		},
		_ => Args::parse()
	};
	let dir = env::current_dir().unwrap();

	let md = input::read_file(dir.join("src").join("lib.rs")).expect("Unable to read file");
	let mut out = File::create(args.out).expect("Unable to create output file");
	output::emit(&md, &mut out).expect("Unable to write output file");
}
