//! Cargo features pick which grammars a build carries.

use slopvac_comments::{Language, extract};

#[test]
fn a_language_is_enabled_exactly_when_its_feature_is() {
    assert_eq!(Language::Python.is_enabled(), cfg!(feature = "lang-python"));
    assert_eq!(Language::Rust.is_enabled(), cfg!(feature = "lang-rust"));
    assert_eq!(
        Language::Tsx.is_enabled(),
        cfg!(feature = "lang-typescript")
    );
}

#[cfg(feature = "lang-python")]
#[test]
fn an_enabled_language_extracts() {
    assert_eq!(
        extract(Language::Python, "# hi\n").expect("extracts").len(),
        1
    );
}

#[cfg(not(feature = "lang-go"))]
#[test]
fn a_disabled_language_names_the_feature_to_enable() {
    let error = extract(Language::Go, "// hi\n").expect_err("not built");
    assert!(
        matches!(
            error,
            slopvac_comments::ExtractError::Disabled {
                feature: "lang-go",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn every_language_names_its_own_feature() {
    for language in Language::all() {
        assert!(language.feature().starts_with("lang-"), "{language:?}");
    }
}
