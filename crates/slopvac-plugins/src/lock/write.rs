use std::fmt::Write as _;

use super::model::Lockfile;

/// A TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                write!(out, "\\u{:04X}", u32::from(c)).ok();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(super) fn write(lock: &Lockfile) -> String {
    let mut out = String::from("schema_version = 3\n");
    if lock.packs().is_empty() {
        out.push_str("packs = []\n");
    }
    for pack in lock.packs() {
        write!(
            out,
            "\n[[packs]]\nid = {}\nkind = {}\nversion = {}\nslopvac_api = {}\nsource = {}\ndigest = {}\n",
            quote(pack.id.as_str()),
            quote(pack.kind.as_str()),
            quote(&pack.version.to_string()),
            pack.slopvac_api,
            quote(&pack.source.to_string()),
            quote(pack.digest.as_str()),
        ).ok();
        if !pack.requires.is_empty() {
            let ids: Vec<String> = pack.requires.iter().map(|id| quote(id.as_str())).collect();
            writeln!(out, "requires = [{}]", ids.join(", ")).ok();
        }
        if let Some(sig) = &pack.signature {
            write!(
                out,
                "\n[packs.signature]\nkey_id = {}\nnot_before = {}\n",
                quote(sig.key_id.as_str()),
                quote(sig.not_before.as_str()),
            )
            .ok();
        }
    }
    out
}
