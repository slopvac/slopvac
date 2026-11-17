//! ZZZ inner doc
/// ZZZ outer doc
/// ZZZ outer two
fn f() -> &'static str {
    // ZZZ plain
    let s = "ZZZ // ZZZ not a comment"; /* ZZZ block */
    s // ZZZ trailing
}

/*
 * ZZZ star line
 * ZZZ star two
 */
/** ZZZ doc block */
fn g() {}
