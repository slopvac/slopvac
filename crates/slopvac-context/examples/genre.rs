//! Prints the genre of each file given, as `slopvac genre` will.
//!
//! Usage: `cargo run -p slopvac-context --example genre -- <files...>`

#[expect(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "example binary reports on the standard streams"
)]
fn main() {
    let matches = slopvac_context::command()
        .get_matches_from(std::iter::once("genre".to_owned()).chain(std::env::args().skip(1)));
    match slopvac_context::run(&matches) {
        Ok(text) => print!("{text}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    }
}
