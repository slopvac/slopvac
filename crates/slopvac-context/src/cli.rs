//! `slopvac genre <file>...`: print the genre of each file with scores and reasons.
//!
//! The CLI epic wires [`command`] and [`run`] into the clap tree.

use crate::append_line;

use clap::{Arg, ArgAction, ArgMatches, Command};

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

/// Render one result as text.
#[must_use]
pub fn render(path: &str, r: &GenreResult) -> String {
    let source = match r.source {
        GenreSource::Explicit => "explicit",
        GenreSource::Detected => "detected",
        GenreSource::Default => "default",
    };
    let mut out = format!("{path}: {} ({source})\n", r.id);
    for reason in &r.reasons {
        append_line(
            &mut out,
            format_args!("  {}: {}", reason.signal, reason.detail),
        );
    }
    let scores: Vec<String> = r.scores.iter().map(|(id, s)| format!("{id} {s}")).collect();
    append_line(&mut out, format_args!("  scores: {}", scores.join(", ")));
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
            let (glob, genre) = raw
                .split_once('=')
                .ok_or_else(|| Error::Parse(format!("`--assign {raw}` needs GLOB=GENRE")))?;
            Ok(GenreAssignment {
                files: vec![glob.to_owned()],
                genre: genre.to_owned(),
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
    let src = std::fs::read_to_string(path).map_err(|e| Error::Parse(format!("{path}: {e}")))?;
    let doc = PlainDocument::from_markdown(path, &src);
    let view: &dyn DocumentView = &doc;
    let result = registry.resolve(
        view,
        &ResolveOptions {
            assignments,
            relative_path: None,
        },
    )?;
    Ok(render(path, &result))
}
