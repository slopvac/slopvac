//! Host conformance tests (plugin-protocol.md section 12.1) against a scripted fake plugin.
#![cfg(unix)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_plugins::hooks::{
    CompleteItem, Completion, Document, Finding, FindingsDocument, RecipeAction, RecipeFile,
    Rendered, Segment, Span,
};
use slopvac_plugins::{
    BudgetCaps, DrainHook, HookKind, HostConfig, Plugin, PluginError, PluginSpec, ReadHook,
    RunBudget, SpawnGate,
};

#[test]
fn a_well_behaved_plugin_tags_documents_and_shuts_down_cleanly() {
    let p = start("ok");
    assert_eq!(p.negotiated().info.name, "fake");
    let out = p.tag("main", &docs(2)).unwrap();
    assert_eq!(out.len(), 2);
    let tags = out[0].outcome.as_ref().unwrap();
    assert_eq!(tags[0].kind, "code-fence");
    assert_eq!((tags[0].span.start, tags[0].span.end), (0, 6));
    let down = p.shutdown();
    assert!(down.clean(), "{down:?}");
}

#[test]
fn h3_stray_stdout_is_a_framing_error() {
    let r = Plugin::start(&spec("stray-stdout"), &cfg(), &Allow);
    assert_eq!(code(&r), "plugin-protocol-violation");
}

#[test]
fn h4_silent_initialize_times_out() {
    let mut c = cfg();
    c.timeouts.initialize = Duration::from_millis(400);
    let started = Instant::now();
    let r = Plugin::start(&spec("silent-init"), &c, &Allow);
    assert_eq!(code(&r), "plugin-timeout");
    assert!(started.elapsed() < NO_HANG);
}

#[test]
fn h5_version_outside_the_host_range_is_not_loaded() {
    let r = Plugin::start(&spec("version-unsupported"), &cfg(), &Allow);
    assert_eq!(code(&r), "protocol-version-unsupported");
    let r = Plugin::start(
        &spec_with("ok", &json!({"protocolVersion": 2}), None, decl()),
        &cfg(),
        &Allow,
    );
    assert_eq!(code(&r), "protocol-version-unsupported");
}

/// Start against a plugin that answers `initialize` with an error object.
fn init_error(numeric: i64, label: &str) -> PluginError {
    let mode = format!("init-error:{numeric}:{label}");
    Plugin::start(&spec(&mode), &cfg(), &Allow)
        .unwrap_err()
        .error
}

#[rstest]
#[case::plugin_range_claims_the_version_label(-32060, "protocol-version-unsupported")]
#[case::host_reserved_claims_the_version_label(-32010, "protocol-version-unsupported")]
fn h5b_an_initialize_error_with_another_number_is_not_a_version_refusal(
    #[case] numeric: i64,
    #[case] label: &str,
) {
    let err = init_error(numeric, label);
    let PluginError::Rpc(e) = &err else {
        panic!("expected the plain RPC error, got {err:?}");
    };
    assert_eq!(e.code, numeric);
    assert_eq!(err.slopvac_code(), "internal-error");
}

#[rstest]
#[case::another_valid_label("my-own-label")]
#[case::a_host_label("plugin-untrusted")]
fn h5c_minus_32001_is_a_version_refusal_whatever_label_it_carries(#[case] label: &str) {
    assert_eq!(
        init_error(-32001, label),
        PluginError::ProtocolVersionUnsupported {
            host: "1..=1".into(),
            plugin: "[3,4]".into(),
        }
    );
}

#[rstest]
#[case::plugin_range(-32060, "plugin-untrusted", "plugin error -32060: m (internal-error)")]
#[case::offline(-32007, "my-own-label", "plugin error -32007: m (offline)")]
#[case::undefined(5, "plugin-hash-mismatch", "plugin error 5: m (internal-error)")]
fn a_displayed_rpc_error_names_the_contract_code_never_the_plugins_label(
    #[case] numeric: i64,
    #[case] label: &str,
    #[case] want: &str,
) {
    let obj = json!({"code": numeric, "message": "m", "data": {"slopvac_code": label}});
    let e = slopvac_plugins::hooks::parse_error_object(&obj).unwrap();
    assert_eq!(PluginError::Rpc(e).to_string(), want);
}

#[test]
fn h6_required_feature_the_host_lacks_stops_the_load_before_spawn() {
    let mut d = decl();
    d.features.required = vec!["telepathy".into()];
    let mut s = spec_with("ok", &json!({}), None, d);
    s.program = PathBuf::from("/nonexistent/never-spawned");
    let err = Plugin::start(&s, &cfg(), &Allow).unwrap_err().error;
    assert_eq!(err, PluginError::RequiredFeatureMissing("telepathy".into()));
}

#[test]
fn h6_unknown_private_optional_feature_loads() {
    let mut d = decl();
    d.features.optional = vec!["x-private".into()];
    let o = json!({"capabilities": {"features": {"optional": ["x-private"]}}});
    assert!(Plugin::start(&spec_with("ok", &o, None, d), &cfg(), &Allow).is_ok());
}

#[test]
fn h7_unknown_hook_kinds_are_ignored_and_known_ones_work() {
    let o = json!({"capabilities": {"hooks": [
        {"kind": "tagger", "id": "main"}, {"kind": "telepath", "id": "x"}]}});
    let p = Plugin::start(&spec_with("ok", &o, None, decl()), &cfg(), &Allow).unwrap();
    assert_eq!(p.negotiated().hooks.len(), 1);
    assert!(p.tag("main", &docs(1)).is_ok());
}

#[test]
fn h8_a_hook_the_plugin_never_offered_is_not_called() {
    let p = start("ok");
    let r = p.call(HookKind::Scorer, "main", json!({}));
    assert_eq!(code(&r), "plugin-usage");
    let r = p.call(HookKind::Tagger, "ghost", json!({}));
    assert_eq!(code(&r), "plugin-usage");
}

#[test]
fn h10_a_missing_document_rejects_the_whole_result() {
    let p = start("missing-doc");
    assert_eq!(code(&p.tag("main", &docs(3))), "plugin-output-invalid");
}

#[test]
fn h11_a_span_past_the_text_fails_that_document_only() {
    let p = start("bad-span");
    let out = p.tag("main", &docs(2)).unwrap();
    assert_eq!(code(&out[0].outcome), "plugin-output-invalid");
    assert!(out[1].outcome.is_ok());
}

#[test]
fn h12_a_per_document_error_leaves_the_others_standing() {
    let p = start("doc-error");
    let out = p.tag("main", &docs(3)).unwrap();
    assert!(out[0].outcome.is_ok());
    assert_eq!(code(&out[1].outcome), "document-invalid");
    assert!(out[2].outcome.is_ok());
}

#[test]
fn h13_a_request_past_its_deadline_that_ignores_cancel_kills_the_plugin() {
    let p = Plugin::start(&spec("hang"), &tight(), &Allow).unwrap();
    let started = Instant::now();
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
    // hook 800 ms + grace 400 ms, against a plugin that would hang for 300 s
    assert!(started.elapsed() < NO_HANG);
    assert_eq!(
        p.failure().map(|e| e.slopvac_code().to_owned()),
        Some("plugin-timeout".into())
    );
    assert!(
        p.tag("main", &docs(1)).is_err(),
        "a killed plugin is not reused"
    );
}

#[test]
fn h14_a_host_cancel_discards_the_cancelled_answer_without_a_diagnostic() {
    let dir = scratch("h14-cancel");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("cancel-ok", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let r = std::thread::scope(|s| {
        let h = s.spawn(|| p.tag("main", &docs(1)));
        // The request is pending once the plugin has it.
        wait_received(&pidfile, "tagger/tag");
        p.cancel_pending();
        h.join().unwrap()
    });
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(code(&r), "request-cancelled");
    assert!(
        p.failure().is_none(),
        "the plugin answered the cancel and lives on"
    );
}

#[test]
fn h14_a_deadline_stands_even_when_the_plugin_answers_the_cancel() {
    let mut c = tight();
    c.timeouts.cancel_grace = cfg().timeouts.cancel_grace;
    let p = Plugin::start(&spec("cancel-late-answer"), &c, &Allow).unwrap();
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
    assert!(
        p.failure().is_none(),
        "an answer inside the grace period saves the process"
    );
}

#[test]
fn h15_a_plugin_that_exits_mid_request_fails_pending_calls_with_its_status_and_stderr() {
    let p = start("exit-mid");
    let err = p.tag("main", &docs(1)).unwrap_err();
    assert_eq!(err.slopvac_code(), "plugin-exited");
    let text = err.to_string();
    assert!(text.contains("boom"), "{text}");
    assert!(text.contains('3'), "{text}");
    assert!(
        !text.contains('\u{1b}'),
        "terminal escapes must not survive: {text:?}"
    );
    assert!(p.tag("main", &docs(1)).is_err(), "no restart");
}

#[test]
fn h16_a_stderr_flood_does_not_stall_the_plugin() {
    let p = start("stderr-flood");
    assert!(p.tag("main", &docs(1)).is_ok());
}

#[test]
fn tm3_stderr_over_the_cap_kills_the_plugin() {
    let mut c = cfg();
    c.caps.stderr_bytes = 1 << 20;
    let p = Plugin::start(&spec("stderr-flood"), &c, &Allow).unwrap();
    let r = p.tag("main", &docs(1));
    assert_eq!(code(&r), "plugin-protocol-violation");
}

#[test]
fn h17_a_plugin_that_ignores_shutdown_is_killed_with_its_children() {
    let dir = scratch("h17");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("ignore-shutdown", &json!({}), Some(&pidfile), decl()),
        &tight(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    let out = p.shutdown();
    let (killed, reported) = reach(&pids);
    let gone = wait_gone(killed);
    end(reported);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!out.left, "the plugin did not leave on its own");
    assert!(gone, "children {killed:?} survived");
}

/// Start the `children` fixture: pids[0] stays in the group, pids[1] left it.
fn children(name: &str) -> (PathBuf, Plugin, Vec<i32>) {
    let dir = scratch(name);
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("children", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    (dir, p, pids)
}

#[cfg(target_os = "linux")]
#[test]
fn tm4_children_in_and_out_of_the_process_group_die_with_the_plugin() {
    let (dir, p, pids) = children("tm4");
    let out = p.shutdown();
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.clean(), "{out:?}");
    assert!(gone, "children {pids:?} survived");
}

/// Without a pidfd the host cannot bind a process outside the group to its
/// id, so it reports that process instead of signalling it (residual R2).
#[cfg(not(target_os = "linux"))]
#[test]
fn tm4_a_child_outside_the_group_is_reported_and_never_signalled() {
    let (dir, p, pids) = children("tm4-report");
    let out = p.shutdown();
    let in_group_gone = wait_gone(&pids[..1]);
    let state = std::process::Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pids[1].to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&state.stdout).trim().to_owned();
    end(&pids[1..]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(in_group_gone, "the group member {} survived", pids[0]);
    assert_eq!(out.teardown.escaped, [pids[1].unsigned_abs()], "{out:?}");
    assert!(!out.clean(), "{out:?}");
    // Sleeping, not stopped or gone: no signal reached it.
    assert!(state.starts_with('S'), "{state:?}");
}

/// TM-4 as the threat model states it. The double-forked grandchild left the
/// group and lost its parent before the kill, so only the launcher, a child
/// subreaper, still links it to the plugin.
#[cfg(target_os = "linux")]
#[test]
fn tm4_children_in_the_group_after_setsid_and_by_double_fork_die_with_a_hung_plugin() {
    let dir = scratch("tm4-three");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("three-children", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_n(&pidfile, 3);
    drop(p);
    let gone = wait_gone(&pids);
    if !gone {
        end(&pids);
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {pids:?} survived");
}

/// A process name that is not UTF-8 hides nothing from the walk.
#[cfg(target_os = "linux")]
#[test]
fn tm4_a_child_outside_the_group_with_a_name_that_is_not_utf8_dies_with_the_plugin() {
    let dir = scratch("tm4-odd-name");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("odd-name:stay", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_n(&pidfile, 1);
    let out = p.shutdown();
    let gone = wait_gone(&pids);
    if !gone {
        end(&pids);
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "the child {pids:?} survived");
    assert!(out.teardown.is_complete(), "{out:?}");
}

/// The same child, left behind by a plugin that exits on its own, dies in
/// the launcher's cleanup.
#[cfg(target_os = "linux")]
#[test]
fn a_child_with_a_name_that_is_not_utf8_left_by_an_exiting_plugin_dies() {
    let dir = scratch("odd-name-exit");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("odd-name:exit", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_n(&pidfile, 1);
    let gone = wait_gone(&pids);
    let end_at = Instant::now() + NO_HANG;
    while p.failure().is_none() && Instant::now() < end_at {
        std::thread::sleep(Duration::from_millis(10));
    }
    let left = p.teardown();
    if !gone {
        end(&pids);
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "the child {pids:?} survived");
    assert!(left.is_complete(), "{left:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn tm4_a_child_of_a_child_that_left_the_group_dies_with_the_plugin() {
    let dir = scratch("tm4-nested");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("nested", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    // pids[0] called `setsid`; pids[1] is its own child.
    let pids = pids_from(&pidfile);
    drop(p);
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "descendants {pids:?} survived");
}

#[test]
fn tm4_a_hung_leader_that_moved_to_another_process_group_is_killed() {
    let dir = scratch("tm4-moved");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("move-group", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    // The leader moved into the group its child pids[1] heads, then stopped reading.
    let pids = pids_from(&pidfile);
    drop(p);
    let (killed, reported) = reach(&pids);
    let gone = wait_gone(killed);
    end(reported);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "processes {killed:?} survived");
}

/// Each generation is found only once its parent is stopped, one level per
/// walk round, so the walk runs to its fixpoint and not to a round count.
#[cfg(target_os = "linux")]
#[test]
fn tm4_a_chain_of_twelve_generations_outside_the_group_dies_with_the_plugin() {
    let dir = scratch("tm4-chain");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("chain:12", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_n(&pidfile, 12);
    let out = p.shutdown();
    let gone = wait_gone(&pids);
    if !gone {
        end(&pids);
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "generations {pids:?} survived");
    assert!(out.teardown.is_complete(), "{out:?}");
}

#[test]
fn dropping_a_plugin_kills_its_process_group() {
    let (dir, p, pids) = children("drop");
    drop(p);
    let (killed, reported) = reach(&pids);
    let gone = wait_gone(killed);
    end(reported);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {killed:?} survived");
}

#[test]
fn h19_a_network_plugin_does_not_spawn_in_offline_mode() {
    let mut d = decl();
    d.network = true;
    let mut s = spec_with("ok", &json!({}), None, d);
    s.program = PathBuf::from("/nonexistent/never-spawned");
    let mut c = cfg();
    c.offline = true;
    let err = Plugin::start(&s, &c, &Allow).unwrap_err().error;
    assert_eq!(err, PluginError::Offline("fake".into()));
    assert_eq!(err.to_string(), "offline: fake needs the network");
    assert_eq!(err.slopvac_code(), "offline-network-needed");
}

#[test]
fn h20_an_untrusted_or_changed_plugin_does_not_spawn() {
    let mut s = spec("ok");
    s.program = PathBuf::from("/nonexistent/never-spawned");
    for e in [
        PluginError::Untrusted("fake".into()),
        PluginError::HashMismatch("fake".into()),
    ] {
        let err = Plugin::start(&s, &cfg(), &Deny(e.clone()))
            .unwrap_err()
            .error;
        assert_eq!(err, e);
    }
}

#[test]
fn h21_the_plugin_sees_a_path_only_when_document_path_was_granted() {
    let mut d = decl();
    d.features.optional = vec!["document-path".into()];
    let o = json!({"capabilities": {"features": {"optional": ["document-path"]}}});
    let granted = Plugin::start(&spec_with("paths", &o, None, d), &cfg(), &Allow).unwrap();
    let denied = start("paths");
    let mut doc = docs(1);
    doc[0].path = Some("docs/a.md".into());
    let evidence = |p: &Plugin| {
        p.tag("main", &doc).unwrap()[0].outcome.as_ref().unwrap()[0]
            .evidence
            .clone()
    };
    assert_eq!(evidence(&granted), "\"docs/a.md\"");
    assert_eq!(evidence(&denied), "null");
}

#[test]
fn h24_a_response_with_an_unknown_id_is_ignored() {
    let p = start("unknown-id");
    assert!(p.tag("main", &docs(1)).is_ok());
}

#[test]
fn h26_a_reply_wider_than_the_manifest_is_a_declaration_mismatch() {
    let cases = [
        json!({"capabilities": {"requiresNetwork": true}}),
        json!({"capabilities": {"hooks": [{"kind": "tagger", "id": "extra"}]}}),
        json!({"capabilities": {"features": {"required": ["document-path"]}}}),
        json!({"capabilities": {"features": {"optional": ["document-path"]}}}),
    ];
    for o in cases {
        let r = Plugin::start(&spec_with("ok", &o, None, decl()), &cfg(), &Allow);
        assert_eq!(code(&r), "plugin-declaration-mismatch", "{o}");
    }
}

#[test]
fn h26_an_optional_feature_cannot_become_required_in_the_reply() {
    let mut d = decl();
    d.features.optional = vec!["document-path".into()];
    let o = json!({"capabilities": {"features": {"required": ["document-path"]}}});
    let r = Plugin::start(&spec_with("ok", &o, None, d), &cfg(), &Allow);
    assert_eq!(code(&r), "plugin-declaration-mismatch");
}

#[test]
fn h26_a_plugin_may_offer_fewer_hooks_than_it_declared() {
    let o = json!({"capabilities": {"hooks": [{"kind": "tagger", "id": "main"}]}});
    assert!(Plugin::start(&spec_with("ok", &o, None, decl()), &cfg(), &Allow).is_ok());
}

#[test]
fn h27_a_fixer_edit_outside_its_finding_rejects_that_document() {
    let findings = vec![Finding {
        rule: "core:x".into(),
        severity: "warning".into(),
        span: Span { start: 2, end: 4 },
        message: "m".into(),
    }];
    let doc = |id: &str| FindingsDocument {
        document: Document {
            id: id.into(),
            text: "abcdef".into(),
            media_type: None,
            path: None,
        },
        findings: findings.clone(),
    };
    let good = start("ok").fix("main", &[doc("a")]).unwrap();
    assert_eq!(good[0].outcome.as_ref().unwrap()[0].text, "fixed");
    let bad = start("fix-outside").fix("main", &[doc("a")]).unwrap();
    assert_eq!(code(&bad[0].outcome), "plugin-output-invalid");
}

#[test]
fn h30_limits_above_the_host_ceilings_are_rejected_not_clamped() {
    let cases = [
        json!({"limits": {"maxInFlight": 17}}),
        json!({"limits": {"maxMessageBytes": 1u64 << 40}}),
        json!({"limits": {"maxMessageBytes": 100}}),
        json!({"limits": {"maxBatchDocuments": 65}}),
        json!({"limits": {"maxInFlight": 0}}),
    ];
    for o in cases {
        let r = Plugin::start(&spec_with("ok", &o, None, decl()), &cfg(), &Allow);
        assert_eq!(code(&r), "plugin-output-invalid", "{o}");
    }
}

#[test]
fn h32_a_request_the_plugin_never_reads_ends_in_a_timeout_without_blocking_the_host() {
    let o = json!({"limits": {"maxMessageBytes": 8 << 20}});
    let p = Plugin::start(&spec_with("no-read", &o, None, decl()), &tight(), &Allow).unwrap();
    let mut big = docs(1);
    big[0].text = "x".repeat(4 << 20);
    let started = Instant::now();
    assert_eq!(code(&p.tag("main", &big)), "plugin-timeout");
    assert!(started.elapsed() < NO_HANG);
}

#[test]
fn h33_a_frame_that_stops_halfway_ends_in_a_timeout() {
    let p = Plugin::start(&spec("partial-frame"), &tight(), &Allow).unwrap();
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
}

#[test]
fn h1_a_body_over_the_limit_is_refused_before_it_is_read() {
    // The body never comes and the hook timer is 30 s: a host that waited
    // for it would report a timeout, not a violation.
    let p = start("oversize");
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-protocol-violation");
}

#[test]
fn tm2_a_flood_of_unsolicited_messages_kills_the_plugin() {
    let p = start("flood");
    let r = p.tag("main", &docs(1));
    assert_eq!(code(&r), "plugin-protocol-violation");
}

/// Threat model 5.5: the cap counts every byte read from standard output.
/// 40 frames carry about 1.8 KB of bodies and about 290 KB of headers, so
/// only the headers can pass a 64 KB cap.
#[test]
fn tm5_5_header_bytes_count_toward_the_standard_output_cap() {
    let mut c = cfg();
    c.caps.stdout_bytes = 64 << 10;
    let p = Plugin::start(&spec("header-padding"), &c, &Allow).unwrap();
    let r = p.tag("main", &docs(1));
    assert_eq!(code(&r), "plugin-protocol-violation");
}

#[test]
fn tm8_the_plugin_environment_holds_only_the_allowlist() {
    let p = start("env");
    let out = p.tag("main", &docs(1)).unwrap();
    let env: Value = serde_json::from_str(&out[0].outcome.as_ref().unwrap()[0].evidence).unwrap();
    let mut keys: Vec<&str> = env
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    // The Python runtime and macOS add these two.
    keys.retain(|k| *k != "LC_CTYPE" && *k != "__CF_USER_TEXT_ENCODING");
    assert_eq!(keys, ["ALLOWED_VAR", "LANG", "PATH"]);
    assert_eq!(env["PATH"], "/usr/bin:/bin");
}

#[test]
fn h15_an_exit_is_reported_at_once_when_a_child_keeps_standard_output_open() {
    let dir = scratch("h15-orphan");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("orphan-tag", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let err = p.tag("main", &docs(1)).unwrap_err();
    let pids = pids_from(&pidfile);
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(err.slopvac_code(), "plugin-exited", "{err}");
    // The hook timer is 30 s: `plugin-exited` came from the exit, not the timer.
    assert!(err.to_string().contains('3'), "{err}");
    assert!(gone, "processes {pids:?} survived");
}

#[test]
fn tm5_the_lifetime_cap_kills_an_idle_plugin_and_its_children() {
    let dir = scratch("tm5");
    let pidfile = dir.join("pids");
    let mut c = cfg();
    // Long enough for the handshake under load; nothing else ends the plugin.
    c.caps.wall_clock = Duration::from_secs(3);
    let p = Plugin::start(
        &spec_with("children", &json!({}), Some(&pidfile), decl()),
        &c,
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    let (killed, reported) = reach(&pids);
    let gone = wait_gone_within(killed, NO_HANG);
    end(reported);
    let code = p.failure().map(|e| e.slopvac_code().to_owned());
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {killed:?} survived the cap");
    assert_eq!(code.as_deref(), Some("plugin-timeout"));
    assert!(p.tag("main", &docs(1)).is_err());
}

#[test]
fn tm5_the_lifetime_cap_fails_a_request_that_is_waiting() {
    let dir = scratch("tm5-waiting");
    let pidfile = dir.join("pids");
    let mut c = cfg();
    c.caps.wall_clock = Duration::from_secs(3);
    c.timeouts.hook = Duration::from_mins(5);
    let p = Plugin::start(
        &spec_with("hang", &json!({}), Some(&pidfile), decl()),
        &c,
        &Allow,
    )
    .unwrap();
    let err = p.tag("main", &docs(1)).unwrap_err();
    let got = received(&pidfile);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        got.iter().any(|m| m["method"] == "tagger/tag"),
        "the request was in flight"
    );
    assert_eq!(err.slopvac_code(), "plugin-timeout");
    assert!(err.to_string().contains("wall-clock cap"), "{err}");
}

#[test]
fn h13_the_hung_process_is_dead_after_the_kill() {
    let dir = scratch("h13");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("hang", &json!({}), Some(&pidfile), decl()),
        &tight(),
        &Allow,
    )
    .unwrap();
    let pids = pids_n(&pidfile, 1);
    assert!(alive(pids[0]));
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "hung plugin {pids:?} survived");
}

#[test]
fn tm11_hostile_stderr_reaches_the_error_as_visible_escapes() {
    let p = start("exit-hostile");
    let text = p.tag("main", &docs(1)).unwrap_err().to_string();
    let want = "\nstderr:\nx\\u{1b}]8;;https://evil.example\\u{7}link\\u{1b}]8;;\\u{7}\\u{9b}31m\\u{202e}RTL\\u{200b}\n\\u{3a}:error::injected\nkept\ttab";
    assert!(text.ends_with(want), "{text:?}");
}

#[test]
fn h16_only_the_tail_of_a_long_stderr_reaches_the_error() {
    let p = start("stderr-tail");
    let text = p.tag("main", &docs(1)).unwrap_err().to_string();
    assert!(text.contains("line-19999"), "the newest line is kept");
    assert!(!text.contains("line-00000"), "the oldest line is dropped");
    // 4 KiB shown at 11 bytes a line, plus the header text.
    assert!(text.len() < 4096 + 200, "{} bytes", text.len());
}

#[test]
fn h1_the_body_limit_is_exact_before_and_after_initialize() {
    // (mode, negotiated maxMessageBytes, passes)
    let cases: [(&str, u64, bool); 6] = [
        ("init-body:1048576", 1 << 20, true),
        ("init-body:1048577", 1 << 20, false),
        ("body:65536", 65_536, true),
        ("body:65537", 65_536, false),
        // Larger than the pre-initialize 1 MiB, inside the negotiated 4 MiB.
        ("body:2097152", 4 << 20, true),
        ("body:4194305", 4 << 20, false),
    ];
    for (mode, negotiated, passes) in cases {
        let o = json!({"limits": {"maxMessageBytes": negotiated}});
        let r = Plugin::start(&spec_with(mode, &o, None, decl()), &cfg(), &Allow);
        let r = r
            .map_err(|e| e.error)
            .and_then(|p| p.tag("main", &docs(1)).map(|_| ()));
        if passes {
            assert!(r.is_ok(), "{mode}: {r:?}");
        } else {
            assert_eq!(code(&r), "plugin-protocol-violation", "{mode}");
        }
    }
}

#[test]
fn h23_documents_over_the_message_limit_go_out_as_two_requests() {
    let o = json!({"limits": {"maxMessageBytes": 65_536}});
    let p = Plugin::start(&spec_with("count", &o, None, decl()), &cfg(), &Allow).unwrap();
    let mut two = docs(2);
    for d in &mut two {
        d.text = "y".repeat(40_000);
    }
    let out = p.tag("main", &two).unwrap();
    let evidence = |i: usize| out[i].outcome.as_ref().unwrap()[0].evidence.clone();
    // The plugin counts the tagger requests it received.
    assert_eq!((evidence(0).as_str(), evidence(1).as_str()), ("1", "2"));
}

#[test]
fn h24_a_second_response_to_a_finished_id_is_ignored() {
    let p = start("duplicate-response");
    assert!(p.tag("main", &docs(1)).is_ok());
    assert!(p.tag("main", &docs(1)).is_ok(), "the plugin lives on");
    assert!(p.failure().is_none());
}

#[test]
fn h25_a_top_level_array_is_a_protocol_violation() {
    let p = start("top-array");
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-protocol-violation");
}

#[test]
fn h35_integer_fields_given_as_floats_or_past_2_pow_53_are_rejected() {
    let typed = [
        json!({"limits": {"maxInFlight": 1.0}}),
        json!({"limits": {"maxBatchDocuments": 64.0}}),
        json!({"limits": {"maxMessageBytes": 1_048_576.0}}),
    ];
    for o in typed {
        let r = Plugin::start(&spec_with("ok", &o, None, decl()), &cfg(), &Allow);
        assert_eq!(code(&r), "plugin-output-invalid", "{o}");
    }
    let o = json!({"limits": {"maxMessageBytes": 9_007_199_254_740_992u64}});
    let r = Plugin::start(&spec_with("ok", &o, None, decl()), &cfg(), &Allow);
    assert_eq!(code(&r), "plugin-protocol-violation");
}

#[test]
fn h34_request_ids_keep_counting_after_completion() {
    let p = start("ids");
    let id = |p: &Plugin| {
        p.tag("main", &docs(1)).unwrap()[0]
            .outcome
            .as_ref()
            .unwrap()[0]
            .evidence
            .clone()
    };
    // `initialize` took id 1.
    assert_eq!([id(&p), id(&p), id(&p)], ["2", "3", "4"]);
}

#[test]
fn h34_a_cancelled_request_id_is_not_reused() {
    let dir = scratch("h34-cancel");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("ids-cancel", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let first = std::thread::scope(|s| {
        let h = s.spawn(|| p.tag("main", &docs(1)));
        wait_received(&pidfile, "tagger/tag");
        p.cancel_pending();
        h.join().unwrap()
    });
    assert_eq!(code(&first), "request-cancelled");
    // `initialize` was 1 and the cancelled request 2.
    let next = p.tag("main", &docs(1));
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(next.unwrap()[0].outcome.as_ref().unwrap()[0].evidence, "3");
}

#[test]
fn standard_output_ending_before_the_exit_does_not_spoil_a_clean_shutdown() {
    // The fixture closes standard output, then exits only after the host has
    // closed standard input, so the reader sees end of stream first.
    let out = start("eof-before-exit").shutdown();
    assert!(out.clean(), "{out:?}");
}

/// plugin-protocol.md section 3: after `shutdown` and `exit` a plugin exits with status 0.
#[test]
fn a_plugin_that_acknowledges_shutdown_and_then_exits_3_has_not_shut_down_cleanly() {
    let out = start("exit-3").shutdown();
    assert!(out.acknowledged && out.left, "{out:?}");
    assert_eq!(out.status.and_then(|s| s.code()), Some(3), "{out:?}");
    assert!(!out.clean(), "{out:?}");
}

#[test]
fn a_plugin_killed_at_shutdown_reports_the_kill_as_its_exit_status() {
    use std::os::unix::process::ExitStatusExt;
    // The plugin never reads, so `shutdown` gets no answer and the exit timer kills it.
    let p = Plugin::start(&spec("no-read"), &tight(), &Allow).unwrap();
    let out = p.shutdown();
    assert!(!out.left, "{out:?}");
    assert_eq!(out.status.and_then(|s| s.signal()), Some(9), "{out:?}");
}

#[test]
fn a_plugin_that_refuses_shutdown_has_not_shut_down_cleanly_even_if_it_exits_0() {
    let out = start("refuse-shutdown").shutdown();
    assert!(!out.acknowledged, "{out:?}");
    assert!(out.left, "{out:?}");
    assert_eq!(out.status.and_then(|s| s.code()), Some(0), "{out:?}");
    assert!(!out.clean(), "{out:?}");
}

#[test]
fn a_plugin_that_ended_before_shutdown_has_not_shut_down_cleanly_even_with_status_0() {
    // The leader exits 0 after `initialized`; its child keeps standard output open.
    let p = start("orphan-init");
    let end = Instant::now() + NO_HANG;
    while p.failure().is_none() {
        assert!(Instant::now() < end, "the exit was never reported");
        std::thread::sleep(Duration::from_millis(10));
    }
    let out = p.shutdown();
    assert!(!out.acknowledged, "{out:?}");
    assert!(out.left, "{out:?}");
    assert_eq!(out.status.and_then(|s| s.code()), Some(0), "{out:?}");
    assert!(!out.clean(), "{out:?}");
}

/// A start that fails after the spawn still reports what its teardown left:
/// a child outside the group dies on Linux and is reported elsewhere.
#[test]
fn a_failed_start_reports_what_its_teardown_left() {
    let dir = scratch("start-stray");
    let pidfile = dir.join("pids");
    // The host refuses a reply with `maxInFlight` 0.
    let o = json!({"limits": {"maxInFlight": 0}});
    let err = Plugin::start(
        &spec_with("stray-init", &o, Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap_err();
    let stray = pids_n(&pidfile, 1);
    let gone = if cfg!(target_os = "linux") {
        wait_gone(&stray)
    } else {
        !alive(stray[0])
    };
    end(&stray);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(err.error.slopvac_code(), "plugin-output-invalid", "{err:?}");
    if cfg!(target_os = "linux") {
        assert!(gone, "the child {stray:?} survived");
        assert!(err.teardown.is_complete(), "{err:?}");
    } else {
        assert!(!gone, "the host signalled the child {stray:?}");
        assert_eq!(err.teardown.escaped, [stray[0].unsigned_abs()], "{err:?}");
    }
}

#[test]
fn h1_a_frame_sent_with_the_initialize_reply_meets_the_negotiated_limit() {
    // One write holds the reply and an unsolicited notification of `size`
    // bytes. (negotiated maxMessageBytes, size, passes)
    let cases: [(u64, usize, bool); 2] = [
        // Over the pre-initialize 1 MiB, inside the negotiated 4 MiB.
        (4 << 20, 2 << 20, true),
        // Inside the pre-initialize 1 MiB, over the negotiated 64 KiB.
        (65_536, 100_000, false),
    ];
    for (negotiated, size, passes) in cases {
        let o = json!({"limits": {"maxMessageBytes": negotiated}});
        let mode = format!("init-then:{size}");
        let r = Plugin::start(&spec_with(&mode, &o, None, decl()), &cfg(), &Allow);
        let r = r
            .map_err(|e| e.error)
            .and_then(|p| p.tag("main", &docs(1)).map(|_| ()));
        if passes {
            assert!(r.is_ok(), "{mode}: {r:?}");
        } else {
            assert_eq!(code(&r), "plugin-protocol-violation", "{mode}");
        }
    }
}

/// Start the fixture offering every hook kind, logging what it receives.
fn typed(name: &str) -> (Plugin, PathBuf) {
    let dir = scratch(name);
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("ok", &offer_all(), Some(&pidfile), decl_all()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    (p, pidfile)
}

/// The params of the one request with `method` the fixture received.
fn sent_params(pidfile: &std::path::Path, method: &str) -> Value {
    let got: Vec<Value> = received(pidfile)
        .into_iter()
        .filter(|m| m["method"] == method)
        .collect();
    assert_eq!(got.len(), 1, "{method}: {got:?}");
    got[0]["params"].clone()
}

fn finding(start: u64, end: u64) -> Finding {
    Finding {
        rule: "core:x".into(),
        severity: "warning".into(),
        span: Span { start, end },
        message: "m".into(),
    }
}

#[test]
fn extract_returns_the_segments_of_each_document() {
    let (p, pidfile) = typed("extract");
    let out = p.extract("main", &docs(1)).unwrap();
    let sent = sent_params(&pidfile, "extractor/extract");
    std::fs::remove_dir_all(pidfile.parent().unwrap()).unwrap();
    let want = Segment {
        span: Span { start: 0, end: 4 },
        context: "prose".into(),
        text: Some("TEXT".into()),
    };
    assert_eq!(out[0].outcome.as_ref().unwrap(), &vec![want]);
    assert_eq!(
        sent,
        json!({"extractor": "main", "documents": [{"id": "d0", "text": "text 0"}]})
    );
}

#[test]
fn score_returns_a_score_per_document_and_none_where_the_scorer_skips() {
    let (p, pidfile) = typed("score");
    let mut d = docs(2);
    d[0].text = "abcdef".into();
    let input = vec![
        FindingsDocument {
            document: d[0].clone(),
            findings: vec![finding(0, 2), finding(3, 4)],
        },
        FindingsDocument {
            document: d[1].clone(),
            findings: vec![],
        },
    ];
    let out = p.score("main", &input).unwrap();
    let sent = sent_params(&pidfile, "scorer/score");
    std::fs::remove_dir_all(pidfile.parent().unwrap()).unwrap();
    // 1.5 per finding.
    assert_eq!(out[0].outcome, Ok(Some(3.0)));
    assert_eq!(out[1].outcome, Ok(None));
    let f = |s: u64, e: u64| json!({"rule": "core:x", "severity": "warning", "span": {"start": s, "end": e}, "message": "m"});
    assert_eq!(
        sent,
        json!({"scorer": "main", "documents": [
            {"id": "d0", "text": "abcdef", "findings": [f(0, 2), f(3, 4)]},
            {"id": "d1", "text": "text 1", "findings": []},
        ]})
    );
}

#[test]
fn render_returns_the_reporter_output() {
    let (p, pidfile) = typed("render");
    let report = json!({"schema_version": 3});
    let options = json!({"width": 80});
    let out = p.render("main", &report, &options).unwrap();
    let sent = sent_params(&pidfile, "reporter/render");
    std::fs::remove_dir_all(pidfile.parent().unwrap()).unwrap();
    assert_eq!(
        out,
        Rendered {
            output: "3:80".into(),
            media_type: "text/plain".into(),
        }
    );
    assert_eq!(
        sent,
        json!({"reporter": "main", "report": {"schema_version": 3}, "options": {"width": 80}})
    );
}

#[test]
fn complete_returns_an_answer_or_an_error_per_item() {
    let (p, pidfile) = typed("complete");
    let items = [
        CompleteItem {
            id: "i1".into(),
            text: "abc".into(),
            instruction: None,
        },
        CompleteItem {
            id: "i2".into(),
            text: "x".into(),
            instruction: Some("fail".into()),
        },
    ];
    let out = p.complete("main", "rewrite", "m1", &items).unwrap();
    let sent = sent_params(&pidfile, "provider/complete");
    std::fs::remove_dir_all(pidfile.parent().unwrap()).unwrap();
    assert_eq!(
        out[0],
        Ok(Completion {
            item: "i1".into(),
            text: "cba".into(),
            model: "m1-echo".into(),
        })
    );
    assert_eq!(out[1].as_ref().unwrap_err().code_name(), "offline");
    assert_eq!(
        sent,
        json!({"provider": "main", "task": "rewrite", "model": "m1", "items": [
            {"id": "i1", "text": "abc"},
            {"id": "i2", "text": "x", "instruction": "fail"},
        ]})
    );
}

#[test]
fn recipe_returns_the_planned_files() {
    let (p, pidfile) = typed("recipe");
    let out = p
        .recipe("main", "claude", &json!({"marker": "hi"}))
        .unwrap();
    let sent = sent_params(&pidfile, "recipe/plan");
    std::fs::remove_dir_all(pidfile.parent().unwrap()).unwrap();
    assert_eq!(
        out,
        vec![
            RecipeFile {
                path: "claude/AGENTS.md".into(),
                action: RecipeAction::Write,
                content: Some("hi".into()),
            },
            RecipeFile {
                path: "old.md".into(),
                action: RecipeAction::Remove,
                content: None,
            },
        ]
    );
    assert_eq!(
        sent,
        json!({"recipe": "main", "harness": "claude", "options": {"marker": "hi"}})
    );
}

/// A config whose plugins share one budget with the given aggregate caps.
fn budgeted(caps: BudgetCaps) -> HostConfig {
    let mut c = cfg();
    c.caps.unsolicited_messages = 100_000;
    c.budget = RunBudget::shared(caps);
    c
}

/// Poll until one of `plugins` has failed; its error code.
fn first_failure_code(plugins: &[&Plugin]) -> String {
    let t = Instant::now();
    while t.elapsed() < NO_HANG {
        if let Some(e) = plugins.iter().find_map(|p| p.failure()) {
            return e.slopvac_code().to_owned();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no plugin failed within {NO_HANG:?}");
}

fn refused_start_code(c: &HostConfig) -> String {
    code(&Plugin::start(&spec("ok"), c, &Allow))
}

/// Each flood plugin sends 1,500 unsolicited messages, under the per-plugin cap of
/// the test config. Two are 3,000 in all, over the aggregate cap of 2,000.
#[test]
fn tm2b_unsolicited_messages_add_up_across_plugins() {
    let c = budgeted(BudgetCaps {
        unsolicited_messages: 2000,
        ..BudgetCaps::default()
    });
    let first = Plugin::start(&spec("flood"), &c, &Allow).unwrap();
    assert!(
        first.tag("main", &docs(1)).is_ok(),
        "1,500 are within 2,000"
    );
    let second = Plugin::start(&spec("flood"), &c, &Allow).unwrap();
    assert_eq!(
        code(&second.tag("main", &docs(1))),
        "plugin-protocol-violation"
    );
    assert_eq!(refused_start_code(&c), "plugin-protocol-violation");
}

/// Each header-padding plugin reads about 290 KB, under the per-plugin default.
#[test]
fn tm2b_standard_output_bytes_add_up_across_plugins() {
    let c = budgeted(BudgetCaps {
        stdout_bytes: 400 << 10,
        ..BudgetCaps::default()
    });
    let first = Plugin::start(&spec("header-padding"), &c, &Allow).unwrap();
    assert!(
        first.tag("main", &docs(1)).is_ok(),
        "290 KB is within 400 KB"
    );
    let second = Plugin::start(&spec("header-padding"), &c, &Allow).unwrap();
    assert_eq!(
        code(&second.tag("main", &docs(1))),
        "plugin-protocol-violation"
    );
    assert_eq!(refused_start_code(&c), "plugin-protocol-violation");
}

/// Each stderr-flood plugin writes 10 MiB, under the per-plugin default. Two
/// are 20 MiB, over the aggregate cap of 15 MiB, so one of them dies.
#[test]
fn tm2b_standard_error_bytes_add_up_across_plugins() {
    let c = budgeted(BudgetCaps {
        stderr_bytes: 15 << 20,
        ..BudgetCaps::default()
    });
    let a = Plugin::start(&spec("stderr-flood"), &c, &Allow).unwrap();
    let b = Plugin::start(&spec("stderr-flood"), &c, &Allow).unwrap();
    assert_eq!(first_failure_code(&[&a, &b]), "plugin-protocol-violation");
    assert_eq!(refused_start_code(&c), "plugin-protocol-violation");
}

#[test]
fn separate_budgets_do_not_share_counts() {
    let caps = BudgetCaps {
        unsolicited_messages: 2000,
        ..BudgetCaps::default()
    };
    for _ in 0..2 {
        let c = budgeted(caps);
        let p = Plugin::start(&spec("flood"), &c, &Allow).unwrap();
        assert!(p.tag("main", &docs(1)).is_ok());
    }
}
/// Four plugins send 1,001 unsolicited messages each. The per-plugin cap of
/// 1,000 rejects each 1,001st, and that message still counts: 4 x 1,001 is
/// 4,004, over the aggregate cap of 4,000.
#[test]
fn tm2b_messages_past_the_plugin_cap_still_count_toward_the_run() {
    let mut c = budgeted(BudgetCaps::default());
    c.caps.unsolicited_messages = 1000;
    for _ in 0..4 {
        let p = Plugin::start(&spec("flood:1001"), &c, &Allow).unwrap();
        assert_eq!(code(&p.tag("main", &docs(1))), "plugin-protocol-violation");
    }
    assert_eq!(refused_start_code(&c), "plugin-protocol-violation");
}

/// A body that never completes is read from the pipe, so it counts: 600,000
/// bytes of a 1,000,000-byte body against an aggregate cap of 300,000.
#[test]
fn tm2b_an_incomplete_frame_counts_toward_the_run() {
    let c = budgeted(BudgetCaps {
        stdout_bytes: 300_000,
        ..BudgetCaps::default()
    });
    let p = Plugin::start(&spec("partial-big"), &c, &Allow).unwrap();
    let r = p.tag("main", &docs(1));
    assert_eq!(code(&r), "plugin-protocol-violation");
    assert!(
        r.unwrap_err().to_string().contains("whole-run budget"),
        "the run budget made the kill"
    );
}

/// A gate that holds the start between its checks and the spawn.
struct BlockingGate {
    entered: crossbeam_channel::Sender<()>,
    release: crossbeam_channel::Receiver<()>,
}

impl SpawnGate for BlockingGate {
    fn check(&self, _: &PluginSpec) -> Result<(), PluginError> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        Ok(())
    }
}

/// A start that passed the early budget check and waits in its gate must not
/// spawn once another plugin has tripped the budget.
#[test]
fn tm2b_a_start_blocked_in_its_gate_does_not_spawn_after_a_breach() {
    let c = budgeted(BudgetCaps {
        unsolicited_messages: 10,
        ..BudgetCaps::default()
    });
    let (entered_tx, entered) = crossbeam_channel::bounded(1);
    let (release_tx, release) = crossbeam_channel::bounded(1);
    let gate = BlockingGate {
        entered: entered_tx,
        release,
    };
    let pidfile = std::env::temp_dir().join(format!("slopvac-gate-{}", std::process::id()));
    let _ = std::fs::remove_file(&pidfile);
    let blocked_spec = spec_with("hang", &json!({}), Some(&pidfile), decl());
    std::thread::scope(|s| {
        let blocked = s.spawn(|| Plugin::start(&blocked_spec, &c, &gate));
        entered
            .recv_timeout(NO_HANG)
            .expect("the start reached its gate");
        let flooder = Plugin::start(&spec("flood"), &c, &Allow).unwrap();
        assert_eq!(
            code(&flooder.tag("main", &docs(1))),
            "plugin-protocol-violation"
        );
        release_tx.send(()).unwrap();
        let r = blocked.join().unwrap();
        assert_eq!(code(&r), "plugin-protocol-violation");
    });
    assert!(!pidfile.exists(), "the blocked plugin was spawned");
}

/// The plugin acknowledges shutdown, writes 2 MiB to standard error and exits
/// 0. The aggregate cap is one byte short of that, so the crossing byte is in
/// the last 64 KiB, which sits in the pipe when the plugin leaves.
#[test]
fn tm2b_output_after_the_shutdown_acknowledgement_makes_it_unclean() {
    let c = budgeted(BudgetCaps {
        stderr_bytes: (2 << 20) - 1,
        ..BudgetCaps::default()
    });
    let p = Plugin::start(&spec("stderr-after-exit"), &c, &Allow).unwrap();
    let out = p.shutdown();
    assert!(out.acknowledged, "{out:?}");
    assert!(out.violation.is_some(), "{out:?}");
    assert!(!out.clean(), "{out:?}");
}
/// A hook that holds the stderr reader at its final read of 2 MiB until
/// `release` fires, or for `NO_HANG`.
fn hold_final_stderr_read(release: crossbeam_channel::Receiver<()>) -> ReadHook {
    ReadHook::new(move |total| {
        if total >= 2 << 20 {
            let _ = release.recv_timeout(NO_HANG);
        }
    })
}

/// The plugin writes 2 MiB to standard error and leaves with status 0, while
/// the host has read the last chunk but not charged it. The final charge
/// crosses the cap after the leader is reaped and before the drain ends.
#[test]
fn tm2b_a_cap_crossing_charge_that_lands_during_the_drain_is_reported() {
    let (release_tx, release) = crossbeam_channel::bounded(1);
    let mut c = budgeted(BudgetCaps {
        stderr_bytes: (2 << 20) - 1,
        ..BudgetCaps::default()
    });
    c.stderr_read_hook = Some(hold_final_stderr_read(release));
    // The shutdown has reaped the leader and is entering the drain: release
    // the held read, and start the drain wait only once the run budget shows
    // the crossing charge.
    let budget = Arc::clone(&c.budget);
    c.drain_hook = Some(DrainHook::new(move || {
        release_tx.send(()).unwrap();
        let end = Instant::now() + NO_HANG;
        while budget.breach().is_none() && Instant::now() < end {
            std::thread::yield_now();
        }
    }));
    let p = Plugin::start(&spec("stderr-after-exit"), &c, &Allow).unwrap();
    let out = p.shutdown();
    assert!(
        out.acknowledged && out.left,
        "the exit was voluntary: {out:?}"
    );
    assert!(out.status.is_some_and(|s| s.success()), "{out:?}");
    assert!(out.violation.is_some(), "{out:?}");
    assert!(!out.clean(), "{out:?}");
}

/// The reader stays paused past the drain wait, as if a descendant held the
/// pipe open: the accounting is unfinished, so the shutdown is not clean.
#[test]
fn tm2b_a_reader_that_outlasts_the_drain_wait_makes_the_shutdown_unclean() {
    let (release_tx, release) = crossbeam_channel::bounded(1);
    let mut c = budgeted(BudgetCaps::default());
    c.stderr_read_hook = Some(hold_final_stderr_read(release));
    let p = Plugin::start(&spec("stderr-after-exit"), &c, &Allow).unwrap();
    let out = p.shutdown();
    release_tx.send(()).unwrap();
    assert!(out.acknowledged && out.left, "{out:?}");
    assert!(out.violation.is_none(), "{out:?}");
    assert!(out.accounting_incomplete, "{out:?}");
    assert!(!out.clean(), "{out:?}");
}
