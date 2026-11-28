//! A binary-level row that fails: the release gate's conformance command must exit non-zero.

mod conformance {
    #[test]
    fn n98_a_seeded_row_fails() {
        panic!("seeded failure: the gate must refuse this row");
    }

    #[test]
    fn n98_a_seeded_row_passes() {}
}
