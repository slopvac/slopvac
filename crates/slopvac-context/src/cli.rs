//! `slopvac genre <file>...`: print the genre of each file with scores and reasons.
//!
//! The CLI epic wires [`command`] and [`run`] into the clap tree.

use std::fmt::Write as _;

use clap::{Arg, ArgAction, ArgMatches, Command};
use slopvac_sanitize::line;

use crate::{
    DocumentView, Error, GenreAssignment, GenreRegistry, GenreResult, GenreSource, PlainDocument,
    ResolveOptions, builtin_genres,
};

/// The `genre` subcommand.
#[must_use]
pub fn command() -> Command {
    Command::new("genre")
        .about("Print the genre of each file, with scores and the reasons for the choice")
        .arg(
            Arg::new("files")
                .num_args(1..)
                .required(true)
                .value_name("FILE"),
        )
        .arg(
            Arg::new("assign")
                .long("assign")
                .value_name("GLOB=GENRE")
                .action(ArgAction::Append)
                .help("Assign a genre to files that match a glob, as a `[[genres]]` block would"),
        )
}

/// Render one result as text. Paths, genre ids and reason details come from the document or
/// the config, so each is escaped for a terminal.
#[must_use]
pub fn render(path: &str, r: &GenreResult) -> String {
    let source = match r.source {
        GenreSource::Explicit => "explicit",
        GenreSource::Detected => "detected",
        GenreSource::Default => "default",
    };
    let mut out = format!("{}: {} ({source})\n", line(path), line(&r.id));
    for reason in &r.reasons {
        let _ = writeln!(out, "  {}: {}", line(&reason.signal), line(&reason.detail));
    }
    let scores: Vec<String> = r
        .scores
        .iter()
        .map(|(id, s)| format!("{} {s}", line(id)))
        .collect();
    let _ = writeln!(out, "  scores: {}", scores.join(", "));
    out
}

/// Resolve the genre of `path` against the built-in genres and print the result.
pub fn run(matches: &ArgMatches) -> Result<String, Error> {
    let registry = builtin_genres()?;
    let assignments = parse_assignments(matches)?;
    let mut out = String::new();
    for path in matches.get_many::<String>("files").into_iter().flatten() {
        out.push_str(&explain(&registry, path, &assignments)?);
    }
    Ok(out)
}

fn parse_assignments(matches: &ArgMatches) -> Result<Vec<GenreAssignment>, Error> {
    matches
        .get_many::<String>("assign")
        .into_iter()
        .flatten()
        .map(|raw| {
            let (glob, genre) = raw.split_once('=').ok_or_else(|| {
                Error::Parse(format!("`--assign {}` needs GLOB=GENRE", line(raw)))
            })?;
            Ok(GenreAssignment {
                files: vec![glob.to_string()],
                genre: genre.to_string(),
            })
        })
        .collect()
}

fn explain(
    registry: &GenreRegistry,
    path: &str,
    assignments: &[GenreAssignment],
) -> Result<String, Error> {
    // STUB(slopvac-8l4.5.11): PlainDocument stands in for the real Markdown parser.
    let src = std::fs::read_to_string(path)
        .map_err(|e| Error::Parse(format!("{}: {}", line(path), line(&e.to_string()))))?;
    let doc = PlainDocument::from_markdown(path, &src);
    let result = registry.resolve(
        &doc as &dyn DocumentView,
        &ResolveOptions {
            assignments,
            relative_path: None,
        },
    )?;
    Ok(render(path, &result))
}
