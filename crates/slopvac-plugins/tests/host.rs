//! Host conformance tests (plugin-protocol.md section 12.1) against a scripted fake plugin.
#![cfg(unix)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::*;
use serde_json::{Value, json};
use slopvac_plugins::hooks::{Document, Finding, FindingsDocument, Span};
use slopvac_plugins::{HookKind, Plugin, PluginError};

#[test]
fn a_well_behaved_plugin_tags_documents_and_shuts_down_cleanly() {
    let p = start("ok");
    assert_eq!(p.negotiated().info.name, "fake");
    let out = p.tag("main", &docs(2)).unwrap();
    assert_eq!(out.len(), 2);
    let tags = out[0].outcome.as_ref().unwrap();
    assert_eq!(tags[0].kind, "code-fence");
    assert_eq!((tags[0].span.start, tags[0].span.end), (0, 6));
    assert!(p.shutdown());
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
    assert!(started.elapsed() < Duration::from_secs(3));
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

#[test]
fn h6_required_feature_the_host_lacks_stops_the_load_before_spawn() {
    let mut d = decl();
    d.features.required = vec!["telepathy".into()];
    let mut s = spec_with("ok", &json!({}), None, d);
    s.program = PathBuf::from("/nonexistent/never-spawned");
    let err = Plugin::start(&s, &cfg(), &Allow).unwrap_err();
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
    let p = start("hang");
    let started = Instant::now();
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
    // hook 800 ms + grace 400 ms
    assert!(started.elapsed() < Duration::from_secs(3));
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
    let p = start("cancel-ok");
    let r = std::thread::scope(|s| {
        let h = s.spawn(|| p.tag("main", &docs(1)));
        std::thread::sleep(Duration::from_millis(200));
        p.cancel_pending();
        h.join().unwrap()
    });
    assert_eq!(code(&r), "request-cancelled");
    assert!(
        p.failure().is_none(),
        "the plugin answered the cancel and lives on"
    );
}

#[test]
fn h14_a_deadline_stands_even_when_the_plugin_answers_the_cancel() {
    let p = start("cancel-late-answer");
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
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    assert!(!p.shutdown(), "the plugin did not leave on its own");
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {pids:?} survived");
}

#[test]
fn tm4_children_in_and_out_of_the_process_group_die_with_the_plugin() {
    let dir = scratch("tm4");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("children", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    assert!(p.shutdown());
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {pids:?} survived");
}

#[test]
fn dropping_a_plugin_kills_its_process_group() {
    let dir = scratch("drop");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("children", &json!({}), Some(&pidfile), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    drop(p);
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {pids:?} survived");
}

#[test]
fn h19_a_network_plugin_does_not_spawn_in_offline_mode() {
    let mut d = decl();
    d.network = true;
    let mut s = spec_with("ok", &json!({}), None, d);
    s.program = PathBuf::from("/nonexistent/never-spawned");
    let mut c = cfg();
    c.offline = true;
    let err = Plugin::start(&s, &c, &Allow).unwrap_err();
    assert_eq!(err, PluginError::Offline("fake".into()));
    assert_eq!(err.to_string(), "offline: fake needs the network");
}

#[test]
fn h20_an_untrusted_or_changed_plugin_does_not_spawn() {
    let mut s = spec("ok");
    s.program = PathBuf::from("/nonexistent/never-spawned");
    for e in [
        PluginError::Untrusted("fake".into()),
        PluginError::HashMismatch("fake".into()),
    ] {
        let err = Plugin::start(&s, &cfg(), &Deny(e.clone())).unwrap_err();
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
    let p = Plugin::start(&spec_with("no-read", &o, None, decl()), &cfg(), &Allow).unwrap();
    let mut big = docs(1);
    big[0].text = "x".repeat(4 << 20);
    let started = Instant::now();
    assert_eq!(code(&p.tag("main", &big)), "plugin-timeout");
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn h33_a_frame_that_stops_halfway_ends_in_a_timeout() {
    let p = start("partial-frame");
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
}

#[test]
fn h1_a_body_over_the_limit_is_refused_before_it_is_read() {
    let p = start("oversize");
    let started = Instant::now();
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-protocol-violation");
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "no wait for a body that never comes"
    );
}

#[test]
fn tm2_a_flood_of_unsolicited_messages_kills_the_plugin() {
    let p = start("flood");
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
    let started = Instant::now();
    let err = p.tag("main", &docs(1)).unwrap_err();
    let elapsed = started.elapsed();
    let pids = pids_from(&pidfile);
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(err.slopvac_code(), "plugin-exited", "{err}");
    assert!(err.to_string().contains('3'), "{err}");
    // The hook timer is 800 ms: the answer came from the exit, not the timer.
    assert!(elapsed < Duration::from_millis(700), "{elapsed:?}");
    assert!(gone, "processes {pids:?} survived");
}

#[test]
fn tm5_the_lifetime_cap_kills_an_idle_plugin_and_its_children() {
    let dir = scratch("tm5");
    let pidfile = dir.join("pids");
    let mut c = cfg();
    c.caps.wall_clock = Duration::from_millis(600);
    let p = Plugin::start(
        &spec_with("children", &json!({}), Some(&pidfile), decl()),
        &c,
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&pidfile);
    assert!(p.failure().is_none(), "alive before the cap");
    let gone = wait_gone(&pids);
    let code = p.failure().map(|e| e.slopvac_code().to_owned());
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "children {pids:?} survived the cap");
    assert_eq!(code.as_deref(), Some("plugin-timeout"));
    assert!(p.tag("main", &docs(1)).is_err());
}

#[test]
fn tm5_the_lifetime_cap_fails_a_request_that_is_waiting() {
    let mut c = cfg();
    c.caps.wall_clock = Duration::from_millis(300);
    c.timeouts.hook = Duration::from_secs(20);
    let p = Plugin::start(&spec("hang"), &c, &Allow).unwrap();
    let started = Instant::now();
    assert_eq!(code(&p.tag("main", &docs(1))), "plugin-timeout");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn h13_the_hung_process_is_dead_after_the_kill() {
    let dir = scratch("h13");
    let pidfile = dir.join("pids");
    let p = Plugin::start(
        &spec_with("hang", &json!({}), Some(&pidfile), decl()),
        &cfg(),
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
        let r = r.and_then(|p| p.tag("main", &docs(1)).map(|_| ()));
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
    let p = start("ids-cancel");
    let first = std::thread::scope(|s| {
        let h = s.spawn(|| p.tag("main", &docs(1)));
        std::thread::sleep(Duration::from_millis(200));
        p.cancel_pending();
        h.join().unwrap()
    });
    assert_eq!(code(&first), "request-cancelled");
    // `initialize` was 1 and the cancelled request 2.
    let next = p.tag("main", &docs(1)).unwrap();
    assert_eq!(next[0].outcome.as_ref().unwrap()[0].evidence, "3");
}

#[test]
fn a_compliant_shutdown_is_clean_every_time() {
    // Standard output closing next to the exit must not read as a failure.
    for i in 0..25 {
        assert!(start("ok").shutdown(), "run {i}");
    }
}
