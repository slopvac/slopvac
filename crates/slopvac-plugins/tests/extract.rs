//! Extraction tests: the malicious-archive corpus (TM-43 to TM-47).
#![cfg(test)]

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use slopvac_plugins::extract::{
    Evicted, ExtractError, Limits, TreeHash, clean_staging, evict, extract_targz, tree_hash,
    verify_tree,
};

const GOLDEN: &str = "sha256:649fc82b4e4db008e801244b088e12aa52921acb707eb30efdc6d60930ecdefa";

#[test]
fn a_tree_hash_parses_only_sha256_and_64_lowercase_hex() {
    assert_eq!(TreeHash::parse(GOLDEN).unwrap().as_str(), GOLDEN);
    let hex = &GOLDEN[7..];
    let bad = [
        format!("sha512:{hex}"),
        hex.to_owned(),
        format!("sha256:{}", &hex[..63]),
        format!("sha256:{hex}0"),
        format!("sha256:{}", hex.to_uppercase()),
        format!("sha256:{}g", &hex[..63]),
        format!("sha256:{} ", &hex[..63]),
        "sha256:".to_owned(),
        String::new(),
    ];
    for s in bad {
        assert!(
            matches!(TreeHash::parse(&s), Err(ExtractError::Refused(_))),
            "{s:?}"
        );
    }
}

struct Entry<'a> {
    name: &'a str,
    typ: u8,
    data: &'a [u8],
    mode: u32,
    link: &'a str,
}

fn file<'a>(name: &'a str, data: &'a [u8]) -> Entry<'a> {
    Entry {
        name,
        typ: b'0',
        data,
        mode: 0o644,
        link: "",
    }
}

fn kind<'a>(name: &'a str, typ: u8, link: &'a str) -> Entry<'a> {
    Entry {
        name,
        typ,
        data: b"",
        mode: 0o644,
        link,
    }
}

fn header(name: &[u8], typ: u8, size: usize, mode: u32, link: &[u8]) -> [u8; 512] {
    let mut h = [0u8; 512];
    let n = name.len().min(100);
    h[..n].copy_from_slice(&name[..n]);
    h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
    h[108..115].copy_from_slice(b"0000000");
    h[116..123].copy_from_slice(b"0000000");
    h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    h[136..147].copy_from_slice(b"00000000000");
    h[156] = typ;
    h[157..157 + link.len().min(100)].copy_from_slice(&link[..link.len().min(100)]);
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

fn pad(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(512) * 512, 0);
}

fn tar(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        let name = e.name.as_bytes();
        if name.len() > 100 {
            let mut long = name.to_vec();
            long.push(0);
            out.extend_from_slice(&header(b"././@LongLink", b'L', long.len(), 0o644, b""));
            pad(&mut out, &long);
        }
        out.extend_from_slice(&header(
            name,
            e.typ,
            e.data.len(),
            e.mode,
            e.link.as_bytes(),
        ));
        pad(&mut out, e.data);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

fn archive(entries: &[Entry<'_>]) -> Vec<u8> {
    gz(&tar(entries))
}

struct Store(std::path::PathBuf);

impl Store {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("slopvac-extract-{name}-{}", std::process::id()));
        evict(&p).ok();
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn trees(&self) -> Vec<String> {
        let Ok(rd) = fs::read_dir(&self.0) else {
            return vec![];
        };
        rd.flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("tree-"))
            .collect()
    }
    fn staging_entries(&self) -> usize {
        fs::read_dir(self.0.join(".staging")).map_or(0, Iterator::count)
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        evict(&self.0).ok();
    }
}

fn extract(
    store: &Store,
    bytes: &[u8],
) -> Result<slopvac_plugins::extract::Extracted, ExtractError> {
    extract_targz(bytes, store.path(), &HashSet::new(), Limits::default())
}

fn assert_refused_cleanly(name: &str, bytes: &[u8]) {
    let store = Store::new(name);
    let err = extract(&store, bytes).expect_err(name);
    assert!(
        matches!(err, ExtractError::Refused(_) | ExtractError::Malformed(_)),
        "{name}: {err:?}"
    );
    assert_eq!(store.trees(), Vec::<String>::new());
    assert_eq!(store.staging_entries(), 0, "{name}: staging left behind");
}

#[test]
fn a_good_archive_is_stored_under_its_hash_and_matches_a_recomputation() {
    let store = Store::new("good");
    let exec: HashSet<String> = ["bin/run".to_owned()].into();
    let a = archive(&[
        kind("bin/", b'5', ""),
        file("bin/run", b"#!/bin/sh\n"),
        file("pack.toml", b"name = 'x'\n"),
        file("./docs/readme.md", b"hi"),
    ]);
    let out = extract_targz(a.as_slice(), store.path(), &exec, Limits::default()).unwrap();
    assert_eq!(out.files, 3);
    assert_eq!(
        out.root,
        store.path().join(format!("tree-{}", out.hash.hex()))
    );
    assert_eq!(fs::read(out.root.join("docs/readme.md")).unwrap(), b"hi");
    assert_eq!(tree_hash(&out.root).unwrap(), out.hash);
    verify_tree(&out.root, &out.hash).unwrap();
}

#[test]
fn the_hash_does_not_depend_on_entry_order_or_directory_entries() {
    let a = Store::new("order-a");
    let b = Store::new("order-b");
    let one = archive(&[file("a", b"1"), file("d/b", b"2")]);
    let two = archive(&[kind("d/", b'5', ""), file("d/b", b"2"), file("a", b"1")]);
    assert_eq!(
        extract(&a, &one).unwrap().hash,
        extract(&b, &two).unwrap().hash
    );
}

#[test]
fn different_content_or_the_executable_flag_changes_the_hash() {
    let s = Store::new("diff");
    let base = archive(&[file("run", b"x")]);
    let plain = extract_targz(
        base.as_slice(),
        s.path(),
        &HashSet::new(),
        Limits::default(),
    )
    .unwrap();
    let exec = extract_targz(
        base.as_slice(),
        s.path(),
        &["run".to_owned()].into(),
        Limits::default(),
    )
    .unwrap();
    let edited = extract(&s, &archive(&[file("run", b"y")])).unwrap();
    assert_ne!(plain.hash, exec.hash);
    assert_ne!(plain.hash, edited.hash);
}

#[cfg(unix)]
#[test]
fn archive_modes_are_ignored_and_the_tree_is_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new("modes");
    let a = archive(&[
        Entry {
            name: "f",
            typ: b'0',
            data: b"x",
            mode: 0o4777,
            link: "",
        },
        file("g", b"y"),
    ]);
    let out = extract_targz(
        a.as_slice(),
        store.path(),
        &["g".to_owned()].into(),
        Limits::default(),
    )
    .unwrap();
    let mode = |p: &str| fs::metadata(out.root.join(p)).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode("f"), 0o444, "setuid and write bits must not survive");
    assert_eq!(mode("g"), 0o555);
}

#[test]
fn tm43_paths_that_escape_or_hide_are_refused() {
    let long = "a".repeat(1025);
    let deep = format!("{}f", "d/".repeat(33));
    let bad: Vec<(&str, &str)> = vec![
        ("dotdot", "../x"),
        ("dotdot-mid", "a/../../x"),
        ("absolute", "/etc/passwd"),
        ("drive", "C:/x"),
        ("backslash", "a\\b"),
        ("control", "bad\u{1}name"),
        ("reserved", "dir/con"),
        ("reserved-ext", "nul.txt"),
        ("trailing-dot", "name."),
        ("overlong", &long),
        ("depth", &deep),
        ("empty-segment", "a//b"),
        ("stream", "ab:stream"),
        ("stream-nested", "d/s:x"),
        ("sidecar-name", ".slopvac-exec"),
        ("sidecar-case", ".SlopVac-Exec"),
    ];
    for (label, name) in bad {
        assert_refused_cleanly(label, &archive(&[file(name, b"x")]));
    }
}

#[test]
fn tm43_links_devices_and_fifos_are_refused() {
    for (label, typ) in [
        ("symlink", b'2'),
        ("hardlink", b'1'),
        ("char", b'3'),
        ("block", b'4'),
        ("fifo", b'6'),
    ] {
        assert_refused_cleanly(
            label,
            &archive(&[file("ok", b"x"), kind("evil", typ, "/etc/passwd")]),
        );
    }
}

#[test]
fn tm43_duplicates_and_folded_collisions_are_refused() {
    let nfc = "\u{e9}.txt";
    let nfd = "e\u{301}.txt";
    let cases: Vec<(&str, Vec<Entry<'_>>)> = vec![
        ("duplicate", vec![file("a", b"1"), file("a", b"2")]),
        (
            "case",
            vec![file("Readme.md", b"1"), file("README.MD", b"2")],
        ),
        ("nfc", vec![file(nfc, b"1"), file(nfd, b"2")]),
        ("file-then-dir", vec![file("a", b"1"), file("a/b", b"2")]),
        ("dir-then-file", vec![file("a/b", b"2"), file("a", b"1")]),
        ("dir-case", vec![file("Dir/x", b"1"), file("dir/y", b"2")]),
    ];
    for (label, entries) in cases {
        assert_refused_cleanly(label, &archive(&entries));
    }
}

#[test]
fn tm44_limits_stop_bombs() {
    let zeros = vec![0u8; 20 << 20];
    let ratio = archive(&[file("zeros", &zeros)]);
    assert!(
        ratio.len() < 100 << 10,
        "fixture should compress far past 100 to 1"
    );
    assert_refused_cleanly("ratio", &ratio);

    let store = Store::new("limits");
    let many: Vec<Entry<'_>> = ["a", "b", "c", "d"].iter().map(|n| file(n, b"x")).collect();
    let a = archive(&many);
    let tight = Limits {
        entries: 3,
        ..Limits::default()
    };
    assert!(matches!(
        extract_targz(a.as_slice(), store.path(), &HashSet::new(), tight),
        Err(ExtractError::Refused(_))
    ));

    let noise: Vec<u8> = (0..4096u32)
        .flat_map(|i| i.wrapping_mul(2_654_435_761).to_le_bytes())
        .collect();
    let a = archive(&[file("big", &noise)]);
    let small = Limits {
        expanded_bytes: 1000,
        ..Limits::default()
    };
    assert!(extract_targz(a.as_slice(), store.path(), &HashSet::new(), small).is_err());
    let tiny = Limits {
        compressed_bytes: 100,
        ..Limits::default()
    };
    assert!(extract_targz(a.as_slice(), store.path(), &HashSet::new(), tiny).is_err());
    assert_eq!(store.trees(), Vec::<String>::new());
    assert_eq!(store.staging_entries(), 0);
}

#[test]
fn tm45_a_truncated_archive_leaves_no_entry_and_old_staging_is_cleaned() {
    let store = Store::new("interrupt");
    let a = archive(&[file("a", &vec![7u8; 100_000]), file("b", b"2")]);
    let half = &a[..a.len() / 2];
    assert!(matches!(
        extract(&store, half),
        Err(ExtractError::Malformed(_) | ExtractError::Io { .. })
    ));
    assert_eq!(store.trees(), Vec::<String>::new());
    assert_eq!(store.staging_entries(), 0);

    let leftover = store.path().join(".staging/x-killed");
    fs::create_dir_all(leftover.join("tree")).unwrap();
    assert_eq!(
        clean_staging(store.path(), Duration::from_secs(3600))
            .unwrap()
            .removed,
        0,
        "fresh staging may belong to a live run"
    );
    assert_eq!(
        clean_staging(store.path(), Duration::ZERO).unwrap().removed,
        1
    );
    assert_eq!(store.staging_entries(), 0);
}

#[test]
fn extracting_the_same_archive_twice_reuses_the_stored_tree() {
    let store = Store::new("twice");
    let a = archive(&[file("a", b"1")]);
    let first = extract(&store, &a).unwrap();
    let second = extract(&store, &a).unwrap();
    assert_eq!(first.root, second.root);
    assert_eq!(store.trees().len(), 1);
}

#[cfg(unix)]
#[test]
fn tm46_any_change_to_a_stored_tree_fails_verification() {
    use std::os::unix::fs::PermissionsExt;
    let make = |label: &str| {
        let store = Store::new(label);
        let out = extract(&store, &archive(&[file("a", b"1"), file("d/b", b"2")])).unwrap();
        (store, out)
    };
    let unlock = |p: &Path| fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();

    let (_s, out) = make("tamper-edit");
    unlock(&out.root.join("a"));
    fs::write(out.root.join("a"), b"changed").unwrap();
    assert!(matches!(
        verify_tree(&out.root, &out.hash),
        Err(ExtractError::Tampered { .. })
    ));

    let (_s, out) = make("tamper-add");
    unlock(&out.root);
    fs::write(out.root.join("extra"), b"x").unwrap();
    assert!(matches!(
        verify_tree(&out.root, &out.hash),
        Err(ExtractError::Tampered { .. })
    ));

    let (_s, out) = make("tamper-delete");
    unlock(&out.root);
    fs::remove_file(out.root.join("a")).unwrap();
    assert!(matches!(
        verify_tree(&out.root, &out.hash),
        Err(ExtractError::Tampered { .. })
    ));

    let (_s, out) = make("tamper-symlink");
    unlock(&out.root);
    unlock(&out.root.join("d"));
    fs::remove_dir_all(out.root.join("d")).unwrap();
    std::os::unix::fs::symlink("/etc", out.root.join("d")).unwrap();
    assert!(matches!(
        verify_tree(&out.root, &out.hash),
        Err(ExtractError::Tampered { .. })
    ));

    let (_s, out) = make("tamper-none");
    verify_tree(&out.root, &out.hash).unwrap();
    evict(&out.root).unwrap();
    assert!(!out.root.exists(), "a read-only tree can still be evicted");
}

#[cfg(unix)]
#[test]
fn tm47_the_store_and_staging_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new("perms");
    extract(&store, &archive(&[file("a", b"1")])).unwrap();
    for dir in [store.path().to_owned(), store.path().join(".staging")] {
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700,
            "{}",
            dir.display()
        );
    }
}
fn put(store: &Store, exec: &[&str], entries: &[Entry<'_>]) -> slopvac_plugins::extract::Extracted {
    let set: HashSet<String> = exec.iter().map(|s| (*s).to_owned()).collect();
    extract_targz(
        archive(entries).as_slice(),
        store.path(),
        &set,
        Limits::default(),
    )
    .unwrap()
}

#[test]
fn the_tree_hash_is_a_fixed_value_on_every_platform() {
    // Worked out apart from the code: sha256 over the sorted records
    // `F\0 path \0 exec len(be u64) sha256(content)`. The executable set is the
    // sidecar file `.slopvac-exec` ("bin/run\n"), which is itself a record.
    let store = Store::new("golden");
    let out = put(
        &store,
        &["bin/run"],
        &[file("bin/run", b"#!/bin/sh\n"), file("pack.toml", b"n\n")],
    );
    assert_eq!(out.hash.as_str(), GOLDEN);
    assert_eq!(tree_hash(&out.root).unwrap().as_str(), GOLDEN);
}

#[cfg(unix)]
#[test]
fn tm46_flipping_the_executable_bit_on_disk_fails_verification() {
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new("chmod");
    let out = put(&store, &[], &[file("a", b"1")]);
    fs::set_permissions(&out.root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(out.root.join("a"), fs::Permissions::from_mode(0o555)).unwrap();
    assert!(matches!(
        verify_tree(&out.root, &out.hash),
        Err(ExtractError::Tampered { .. })
    ));
}

#[cfg(unix)]
#[test]
fn tm46_names_that_alias_in_the_hash_are_refused_during_verification() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new("alias");
    let out = put(&store, &[], &[file("d/b", b"2")]);
    let honest = out.hash.clone();
    fs::set_permissions(&out.root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(out.root.join("d"), fs::Permissions::from_mode(0o755)).unwrap();
    // `d\b` as one file name would hash like `d/b` if backslashes were rewritten.
    fs::remove_dir_all(out.root.join("d")).unwrap();
    fs::write(out.root.join("d\\b"), b"2").unwrap();
    assert!(matches!(
        tree_hash(&out.root),
        Err(ExtractError::Refused(_))
    ));
    assert!(matches!(
        verify_tree(&out.root, &honest),
        Err(ExtractError::Tampered { .. })
    ));
    fs::remove_file(out.root.join("d\\b")).unwrap();
    let bad = std::ffi::OsStr::from_bytes(b"x\xff");
    // Some file systems (APFS) refuse non-UTF-8 names outright.
    if fs::write(out.root.join(bad), b"2").is_ok() {
        assert!(matches!(
            tree_hash(&out.root),
            Err(ExtractError::Refused(_))
        ));
        fs::remove_file(out.root.join(bad)).unwrap();
    }
    fs::write(out.root.join("s:t"), b"2").unwrap();
    assert!(matches!(
        tree_hash(&out.root),
        Err(ExtractError::Refused(_))
    ));
}

#[cfg(unix)]
#[test]
fn tm46_a_root_that_is_a_symlink_is_refused() {
    let store = Store::new("rootlink");
    let out = put(&store, &[], &[file("a", b"1")]);
    let link = store.path().join("link");
    std::os::unix::fs::symlink(&out.root, &link).unwrap();
    assert!(matches!(tree_hash(&link), Err(ExtractError::Refused(_))));
    assert!(matches!(
        verify_tree(&link, &out.hash),
        Err(ExtractError::Tampered { .. })
    ));
    // A trailing separator must not make the lstat follow the link.
    for spelling in [
        format!("{}/", link.display()),
        format!("{}/.", link.display()),
    ] {
        assert!(
            matches!(
                tree_hash(Path::new(&spelling)),
                Err(ExtractError::Refused(_))
            ),
            "{spelling}"
        );
    }
}

#[cfg(unix)]
#[test]
fn tm46_verification_bounds_depth_and_size() {
    let store = Store::new("bounds");
    let deep = store.path().join("deep");
    let mut p = deep.clone();
    for _ in 0..40 {
        p.push("d");
    }
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("f"), b"x").unwrap();
    assert!(matches!(tree_hash(&deep), Err(ExtractError::Refused(_))));

    let big = store.path().join("big");
    fs::create_dir_all(&big).unwrap();
    fs::File::create(big.join("f"))
        .unwrap()
        .set_len(300 << 20)
        .unwrap();
    assert!(matches!(tree_hash(&big), Err(ExtractError::Refused(_))));
}

#[test]
fn tm44_a_second_gzip_member_counts_against_the_limits() {
    let store = Store::new("members");
    let mut a = archive(&[file("a", b"1")]);
    a.extend(gz(&vec![0u8; 20 << 20]));
    let err = extract(&store, &a).expect_err("appended member");
    assert!(
        matches!(err, ExtractError::Refused(_) | ExtractError::Malformed(_)),
        "{err:?}"
    );
    assert_eq!(store.trees(), Vec::<String>::new());
}

#[test]
fn tm44_a_large_tail_after_the_tar_end_marker_is_drained_and_limited() {
    let store = Store::new("tail");
    let mut t = tar(&[file("a", b"1")]);
    t.extend(vec![0u8; 20 << 20]);
    assert!(extract(&store, &gz(&t)).is_err());
    assert_eq!(store.trees(), Vec::<String>::new());
    assert_eq!(store.staging_entries(), 0);
}

/// A consistent GNU sparse header: `stored` data bytes in the body (the header
/// size), up to four one-byte descriptors `(offset, length)`, the real size, and
/// the extension flag as given.
fn sparse_header(
    stored: usize,
    real_size: usize,
    extents: &[(usize, usize)],
    extended: bool,
) -> [u8; 512] {
    let mut h = header(b"s", b'S', stored, 0o644, b"");
    h[257..265].copy_from_slice(b"ustar  \0");
    for (i, (off, len)) in extents.iter().enumerate() {
        let at = 386 + 24 * i;
        h[at..at + 12].copy_from_slice(format!("{off:011o}\0").as_bytes());
        h[at + 12..at + 24].copy_from_slice(format!("{len:011o}\0").as_bytes());
    }
    h[482] = u8::from(extended);
    h[483..495].copy_from_slice(format!("{real_size:011o}\0").as_bytes());
    let sum: u32 = h
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u32::from(b)
            }
        })
        .sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

#[test]
fn tm44_a_header_only_sparse_entry_is_refused_before_its_extension_blocks() {
    // The header promises extension blocks and two stored bytes, but the stream
    // ends. Refused, not Malformed: nothing after the header is consumed.
    let store = Store::new("sparse-header");
    let err = extract(&store, &gz(&sparse_header(2, 4, &[(0, 1)], true))).expect_err("sparse");
    assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
}

#[test]
fn tm44_a_well_formed_sparse_archive_is_refused() {
    // Four extents in the header and 100 full extension blocks of 21, so 2104
    // extents of 512 bytes each at offsets 0, 1024, 2048, ... The stored data is
    // 2104 * 512 bytes, already block-aligned, and the extents are back to back
    // in it. The last extent ends at 1024 * 2103 + 512, which is the real size.
    // Every size agrees and nothing overlaps, so only the entry type is at fault.
    const EXTENT: usize = 512;
    let extents = 4 + 100 * 21;
    let offset = |index: usize| 2 * EXTENT * index;
    let head: Vec<(usize, usize)> = (0..4).map(|i| (offset(i), EXTENT)).collect();
    let real_size = offset(extents - 1) + EXTENT;
    let mut t = sparse_header(extents * EXTENT, real_size, &head, true).to_vec();
    for block in 0..100usize {
        let mut ext = [0u8; 512];
        for slot in 0..21usize {
            let index = 4 + block * 21 + slot;
            let at = slot * 24;
            ext[at..at + 12].copy_from_slice(format!("{:011o}\0", offset(index)).as_bytes());
            ext[at + 12..at + 24].copy_from_slice(format!("{EXTENT:011o}\0").as_bytes());
        }
        ext[504] = u8::from(block + 1 < 100);
        t.extend_from_slice(&ext);
    }
    pad(&mut t, &vec![b'x'; extents * EXTENT]);
    t.extend_from_slice(&[0u8; 1024]);

    // The fixture is valid: the ordinary reader, which accepts sparse files,
    // reads it back to the real size.
    let mut reader = tar::Archive::new(t.as_slice());
    let mut entry = reader
        .entries()
        .unwrap()
        .next()
        .expect("one entry")
        .expect("a readable sparse entry");
    assert_eq!(entry.size(), u64::try_from(real_size).unwrap());
    let mut content = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut content).unwrap();
    assert_eq!(content.len(), real_size);
    assert_eq!(content[..EXTENT], [b'x'; EXTENT]);
    assert!(content[EXTENT..2 * EXTENT].iter().all(|&b| b == 0));

    let store = Store::new("sparse-valid");
    let err = extract(&store, &gz(&t)).expect_err("sparse");
    assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
}

#[test]
fn tm44_oversized_pax_metadata_is_refused() {
    let store = Store::new("pax");
    let mut rec = b"1000000 comment=".to_vec();
    rec.resize(999_999, b'x');
    rec.push(b'\n');
    let mut t = Vec::new();
    t.extend_from_slice(&header(b"PaxHeader", b'x', rec.len(), 0o644, b""));
    pad(&mut t, &rec);
    t.extend_from_slice(&header(b"a", b'0', 1, 0o644, b""));
    pad(&mut t, b"1");
    t.extend_from_slice(&[0u8; 1024]);
    assert!(matches!(
        extract(&store, &gz(&t)),
        Err(ExtractError::Refused(_))
    ));
}

#[test]
fn a_pax_path_record_names_the_next_entry() {
    let store = Store::new("paxpath");
    let rec = b"11 path=ok\n";
    let mut t = Vec::new();
    t.extend_from_slice(&header(b"PaxHeader", b'x', rec.len(), 0o644, b""));
    pad(&mut t, rec);
    t.extend_from_slice(&header(b"ignored", b'0', 1, 0o644, b""));
    pad(&mut t, b"1");
    t.extend_from_slice(&[0u8; 1024]);
    let out = extract(&store, &gz(&t)).unwrap();
    assert_eq!(fs::read(out.root.join("ok")).unwrap(), b"1");
}

#[test]
fn reusing_a_stored_tree_leaves_no_staging_directory() {
    let store = Store::new("reuse");
    let a = archive(&[file("d/e/f", b"1"), file("g", b"2")]);
    extract(&store, &a).unwrap();
    extract(&store, &a).unwrap();
    assert_eq!(store.staging_entries(), 0);
}

#[cfg(unix)]
#[test]
fn a_corrupted_cached_tree_is_replaced_on_reinstall() {
    use std::os::unix::fs::PermissionsExt;
    let store = Store::new("heal");
    let a = archive(&[file("a", b"good")]);
    let first = extract(&store, &a).unwrap();
    fs::set_permissions(&first.root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(first.root.join("a"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(first.root.join("a"), b"evil").unwrap();
    let second = extract(&store, &a).unwrap();
    assert_eq!(fs::read(second.root.join("a")).unwrap(), b"good");
    verify_tree(&second.root, &second.hash).unwrap();
    assert_eq!(store.staging_entries(), 0);
}

fn nodes(limit: usize) -> Limits {
    Limits {
        nodes: limit,
        ..Limits::default()
    }
}

fn extract_with(
    store: &Store,
    exec: &[&str],
    files: &[&str],
    limits: Limits,
) -> Result<slopvac_plugins::extract::Extracted, ExtractError> {
    let entries: Vec<Entry<'_>> = files.iter().map(|n| file(n, b"x")).collect();
    let set: HashSet<String> = exec.iter().map(|s| (*s).to_owned()).collect();
    extract_targz(archive(&entries).as_slice(), store.path(), &set, limits)
}

#[test]
fn a_tree_at_the_node_limit_extracts_and_verifies() {
    // Worked out by hand: a/b/c/f (4 nodes) + d/g (2) + h (1) = 7 nodes.
    let store = Store::new("nodes-ok");
    let out = extract_with(&store, &[], &["a/b/c/f", "d/g", "h"], nodes(7)).unwrap();
    verify_tree(&out.root, &out.hash).unwrap();
}

#[test]
fn implicit_parent_directories_count_toward_the_node_limit() {
    // Three archive entries, but seven nodes on disk: refused at 6.
    let store = Store::new("nodes-over");
    let err = extract_with(&store, &[], &["a/b/c/f", "d/g", "h"], nodes(6))
        .expect_err("7 nodes against a limit of 6");
    assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
    assert_eq!(store.trees(), Vec::<String>::new());
    assert_eq!(store.staging_entries(), 0);
}

#[test]
fn shared_parents_are_counted_once() {
    // a, a/b, a/b/f, a/b/g: four nodes.
    let store = Store::new("nodes-shared");
    let out = extract_with(&store, &[], &["a/b/f", "a/b/g"], nodes(4)).unwrap();
    verify_tree(&out.root, &out.hash).unwrap();
}

#[test]
fn the_sidecar_counts_as_a_node() {
    // x, y, e plus the executable list: four nodes.
    let store = Store::new("nodes-sidecar");
    let out = extract_with(&store, &["e"], &["x", "y", "e"], nodes(4)).unwrap();
    verify_tree(&out.root, &out.hash).unwrap();
    let store = Store::new("nodes-sidecar-over");
    let err = extract_with(&store, &["e"], &["x", "y", "e"], nodes(3)).expect_err("4 > 3");
    assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
}
#[cfg(unix)]
mod hostile_cache {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn with_sidecar(name: &str, content: &[u8]) -> (Store, std::path::PathBuf) {
        let store = Store::new(name);
        let out = put(&store, &["run"], &[file("run", b"#!"), file("b", b"1")]);
        let side = out.root.join(".slopvac-exec");
        fs::set_permissions(&side, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&side, content).unwrap();
        (store, out.root)
    }

    #[test]
    fn tm46_a_non_canonical_or_hostile_sidecar_is_refused_while_parsing() {
        let many = "run\n".repeat(2 << 20); // 8 MiB of repeated names
        let long = format!("{}\n", "a".repeat(5000));
        let cases: [(&str, &[u8]); 6] = [
            ("dup", b"run\nrun\n"),
            ("unsorted", b"run\nb\n"),
            ("blank", b"\nrun\n"),
            ("nonewline", b"run"),
            ("flood", many.as_bytes()),
            ("longline", long.as_bytes()),
        ];
        for (name, content) in cases {
            let (_store, root) = with_sidecar(&format!("hostile-{name}"), content);
            assert!(
                matches!(tree_hash(&root), Err(ExtractError::Refused(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn tm46_a_sidecar_that_names_a_missing_file_or_itself_is_refused() {
        for (name, content) in [("missing", "ghost\n"), ("self", ".slopvac-exec\n")] {
            let (_store, root) = with_sidecar(&format!("names-{name}"), content.as_bytes());
            assert!(
                matches!(tree_hash(&root), Err(ExtractError::Refused(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn tm46_a_declared_executable_that_lost_its_bit_fails_verification() {
        let store = Store::new("lostbit");
        let out = put(&store, &["run"], &[file("run", b"#!"), file("b", b"1")]);
        fs::set_permissions(out.root.join("run"), fs::Permissions::from_mode(0o444)).unwrap();
        assert!(matches!(
            verify_tree(&out.root, &out.hash),
            Err(ExtractError::Tampered { .. })
        ));
    }
}
#[test]
fn tm43_unicode_case_folding_collisions_are_refused() {
    let cases: Vec<(&str, Vec<Entry<'_>>)> = vec![
        // Final and medial sigma fold to the same letter.
        (
            "sigma-dir",
            vec![file("\u{3c3}/a", b"1"), file("\u{3c2}/b", b"2")],
        ),
        (
            "sigma-capital",
            vec![file("\u{3a3}", b"1"), file("\u{3c2}", b"2")],
        ),
        // Sharp s folds to "ss".
        (
            "sharp-s",
            vec![file("stra\u{df}e", b"1"), file("STRASSE", b"2")],
        ),
        // Capital sharp s and small sharp s both fold to "ss" under full folding.
        (
            "capital-sharp-s",
            vec![file("\u{1e9e}/a", b"1"), file("\u{df}/b", b"2")],
        ),
        // Kelvin sign folds to k.
        ("kelvin", vec![file("\u{212a}", b"1"), file("k", b"2")]),
        // NTFS uppercases dotless i to I, which default folding keeps apart.
        (
            "dotless-i",
            vec![file("I/a", b"1"), file("\u{131}/b", b"2")],
        ),
    ];
    for (label, entries) in cases {
        assert_refused_cleanly(label, &archive(&entries));
    }
}

#[test]
fn a_gnu_long_name_names_the_next_entry() {
    let store = Store::new("longname");
    let name = format!("d/{}", "n".repeat(150));
    let out = extract(&store, &archive(&[file(&name, b"long")])).unwrap();
    assert_eq!(fs::read(out.root.join(&name)).unwrap(), b"long");
}

fn pax_record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut total = body.len() + 1;
    while total.to_string().len() + body.len() != total {
        total = total.to_string().len() + body.len();
    }
    format!("{total}{body}").into_bytes()
}

#[test]
fn tm44_pax_size_and_sparse_overrides_are_refused() {
    // The record is well formed and the entry after it is an ordinary file, so
    // only the override itself can cause the refusal.
    let cases = [
        ("size", pax_record("size", "100")),
        ("sparse-major", pax_record("GNU.sparse.major", "1")),
        ("sparse-realsize", pax_record("GNU.sparse.realsize", "9")),
    ];
    for (label, rec) in cases {
        let store = Store::new(&format!("pax-{label}"));
        let mut t = Vec::new();
        t.extend_from_slice(&header(b"PaxHeader", b'x', rec.len(), 0o644, b""));
        pad(&mut t, &rec);
        t.extend_from_slice(&header(b"a", b'0', 1, 0o644, b""));
        pad(&mut t, b"1");
        t.extend_from_slice(&[0u8; 1024]);
        let err = extract(&store, &gz(&t)).expect_err(label);
        assert!(matches!(err, ExtractError::Refused(_)), "{label}: {err:?}");
    }
}

#[test]
fn a_node_limit_above_the_verification_ceiling_is_clamped_to_it() {
    // 20 001 one-byte files against a requested limit of usize::MAX. The
    // verification ceiling is 20 000, so extraction must stop first.
    let names: Vec<String> = (0..20_001).map(|i| format!("f{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let limits = Limits {
        nodes: usize::MAX,
        entries: 30_000,
        max_ratio: u64::MAX,
        ..Limits::default()
    };
    let store = Store::new("clamp");
    let err = extract_with(&store, &[], &refs, limits).expect_err("20 001 nodes");
    assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
    assert_eq!(store.trees(), Vec::<String>::new());
}

/// `depth` nested directories below `base/name`, each named `d`.
fn nested(base: &Path, name: &str, depth: usize) -> std::path::PathBuf {
    let top = base.join(name);
    let mut p = top.clone();
    for _ in 0..depth {
        p.push("d");
    }
    fs::create_dir_all(&p).unwrap();
    top
}

fn quarantined_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".quarantine-"))
        .collect();
    v.sort();
    v
}

#[test]
fn a_staging_tree_at_the_depth_bound_is_removed_and_one_past_it_is_quarantined() {
    let store = Store::new("depth");
    let staging = store.path().join(".staging");
    // The root is depth 0; 32 nested levels reach the bound, 33 pass it.
    nested(&staging, "x-at-bound", 32);
    nested(&staging, "x-past-bound", 33);

    let report = clean_staging(store.path(), Duration::ZERO).unwrap();

    assert_eq!(report.removed, 1, "{report:?}");
    assert!(!staging.join("x-at-bound").exists());
    assert!(!staging.join("x-past-bound").exists());
    let aside = quarantined_in(&staging);
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(report.quarantined, vec![staging.join(&aside[0])]);
    // Renamed whole: the deepest directory is still there.
    let mut deepest = staging.join(&aside[0]);
    for _ in 0..33 {
        deepest.push("d");
    }
    assert!(deepest.is_dir());
}

#[test]
fn repeated_staging_cleanup_keeps_existing_quarantines() {
    let store = Store::new("keep-quarantine");
    let staging = store.path().join(".staging");
    nested(&staging, "x-deep", 33);
    clean_staging(store.path(), Duration::ZERO).unwrap();
    let first = quarantined_in(&staging);
    assert_eq!(first.len(), 1);

    let again = clean_staging(store.path(), Duration::ZERO).unwrap();

    assert_eq!(again.removed, 0);
    assert!(again.quarantined.is_empty(), "{again:?}");
    assert_eq!(
        quarantined_in(&staging),
        first,
        "the quarantine was touched"
    );
}

#[test]
fn evict_reports_whether_it_removed_or_quarantined() {
    let store = Store::new("evict-outcome");
    fs::create_dir_all(store.path()).unwrap();
    let small = nested(store.path(), "small", 2);
    let deep = nested(store.path(), "deep", 33);

    assert_eq!(evict(&small).unwrap(), Evicted::Removed);
    assert!(!small.exists());

    let Evicted::Quarantined(aside) = evict(&deep).unwrap() else {
        panic!("a tree past the depth bound must be quarantined");
    };
    assert!(!deep.exists());
    assert!(aside.is_dir());
    assert_eq!(aside.parent(), Some(store.path()));
}

#[cfg(unix)]
mod hostile_store {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    /// An outside directory with an old child tree whose modes and content the
    /// store code must never touch.
    fn outside_with_child(label: &str) -> (Store, std::path::PathBuf) {
        let outside = Store::new(label);
        let child = outside.path().join("victim");
        fs::create_dir_all(child.join("sub")).unwrap();
        fs::write(child.join("sub/f"), b"keep").unwrap();
        fs::set_permissions(&child, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(child.join("sub"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o755)).unwrap();
        (outside, child)
    }

    #[test]
    fn tm47_cleaning_a_staging_directory_that_is_a_symlink_touches_nothing_outside() {
        let (outside, child) = outside_with_child("clean-outside");
        let store = Store::new("clean-linked");
        fs::create_dir_all(store.path()).unwrap();
        symlink(outside.path(), store.path().join(".staging")).unwrap();

        let err = clean_staging(store.path(), Duration::ZERO).expect_err("linked staging");

        assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
        assert_eq!(fs::read(child.join("sub/f")).unwrap(), b"keep");
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&child), 0o755);
        assert_eq!(mode(outside.path()), 0o755);
        assert_eq!(
            fs::read_dir(outside.path()).unwrap().count(),
            1,
            "nothing was quarantined or removed outside the store"
        );
    }

    #[test]
    fn tm47_evicting_a_symlink_removes_the_link_not_its_target() {
        // The plain path and the spellings that make lstat follow the link.
        type Spell = fn(&Path) -> std::path::PathBuf;
        let spellings: [(&str, Spell); 3] = [
            ("plain", |p| p.to_owned()),
            ("slash", |p| format!("{}/", p.display()).into()),
            ("dot", |p| p.join(".")),
        ];
        for (label, spell) in spellings {
            let (outside, child) = outside_with_child(&format!("evict-outside-{label}"));
            let store = Store::new(&format!("evict-linked-{label}"));
            fs::create_dir_all(store.path()).unwrap();
            let link = store.path().join("tree-link");
            symlink(outside.path(), &link).unwrap();

            assert_eq!(evict(&spell(&link)).unwrap(), Evicted::Removed, "{label}");

            assert!(fs::symlink_metadata(&link).is_err(), "{label}");
            assert_eq!(fs::read(child.join("sub/f")).unwrap(), b"keep", "{label}");
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&child), 0o755, "{label}");
            assert_eq!(mode(&child.join("sub")), 0o755, "{label}");
            assert_eq!(mode(outside.path()), 0o755, "{label}");
        }
    }

    #[test]
    fn tm47_a_staging_directory_that_is_a_symlink_is_refused_without_following_it() {
        let outside = Store::new("outside");
        fs::create_dir_all(outside.path()).unwrap();
        fs::set_permissions(outside.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let store = Store::new("linked-staging");
        fs::create_dir_all(store.path()).unwrap();
        symlink(outside.path(), store.path().join(".staging")).unwrap();

        let err = extract(&store, &archive(&[file("a", b"1")])).expect_err("linked staging");
        assert!(matches!(err, ExtractError::Refused(_)), "{err:?}");
        let mode = fs::metadata(outside.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "the target's mode was changed");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert_eq!(store.trees(), Vec::<String>::new());
    }

    #[test]
    fn tm46_an_oversized_cached_tree_is_quarantined_not_walked_on_reinstall() {
        let store = Store::new("oversized");
        let a = archive(&[file("a", b"good")]);
        let first = extract(&store, &a).unwrap();
        // Grow the cached tree past the 20 000 node ceiling.
        fs::set_permissions(&first.root, fs::Permissions::from_mode(0o755)).unwrap();
        for i in 0..20_100 {
            fs::File::create(first.root.join(format!("junk{i}"))).unwrap();
        }
        let second = extract(&store, &a).unwrap();
        assert_eq!(second.root, first.root);
        verify_tree(&second.root, &second.hash).unwrap();
        assert_eq!(fs::read(second.root.join("a")).unwrap(), b"good");
        // The hostile tree was renamed aside whole, not deleted entry by entry.
        let aside: Vec<_> = fs::read_dir(store.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".quarantine-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(
            second.quarantined.as_deref(),
            Some(aside[0].path().as_path())
        );
        assert!(aside[0].path().join("junk0").exists());
        assert!(aside[0].path().join("a").exists());
        assert_eq!(store.staging_entries(), 0);
        fs::remove_dir_all(aside[0].path()).unwrap();
    }
}
