//! Compile-time checks of the derive macro: each file under `tests/ui` is built and its
//! compiler output compared with the `.stderr` beside it.

#[test]
fn the_derive_rejects_bad_catalogs_at_compile_time() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/pass.rs");
    cases.compile_fail("tests/ui/bad_code.rs");
    cases.compile_fail("tests/ui/non_fluent_id.rs");
    cases.compile_fail("tests/ui/missing_field.rs");
    cases.compile_fail("tests/ui/tuple_variant.rs");
    cases.compile_fail("tests/ui/positional_param.rs");
}
