//! OCI client tests against a scripted loopback registry (threat model section 6.4).

mod oci_server;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oci_server::{Request, Response, Server, base64_std, with_auth};
use serde_json::json;
use slopvac_plugins::oci::{
    Anonymous, Client, DefaultCredentials, Descriptor, Digest, EnvCredentials, MEDIA_INDEX,
    MEDIA_MANIFEST, OciError, Reference,
};

fn client_with_basic(user: &str, token: &str) -> Client {
    Client::with_loopback_http_for_tests(Box::new(DefaultCredentials {
        env: EnvCredentials {
            user: Some(user.into()),
            token: Some(token.into()),
            github_token: None,
        },
        ..DefaultCredentials::default()
    }))
}

fn reference(server: &Server, tail: &str) -> Reference {
    Reference::parse(&format!("{}/{tail}", server.registry())).unwrap()
}

fn manifest_json(config: &Descriptor, layers: &[Descriptor]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2, "mediaType": MEDIA_MANIFEST,
        "artifactType": "application/vnd.slopvac.pack.v1",
        "config": config, "layers": layers,
    }))
    .unwrap()
}

fn host(req: &Request) -> String {
    req.header("host").unwrap_or_default().to_owned()
}

fn manifest_response(body: &[u8]) -> Response {
    Response::new(200)
        .header("Content-Type", MEDIA_MANIFEST)
        .body(body)
}

#[test]
fn bearer_flow_fetches_a_token_once_and_reuses_it() {
    let body = manifest_json(
        &Descriptor::for_bytes("application/vnd.oci.empty.v1+json", b"{}"),
        &[],
    );
    let payload = body.clone();
    let server = Server::start(move |req| {
        if req.path.starts_with("/token") {
            return Response::new(200).json(&json!({"token": "T0K", "expires_in": 300}));
        }
        if req.header("authorization") != Some("Bearer T0K") {
            let challenge = format!(
                "Bearer realm=\"http://{}/token\",service=\"reg\",scope=\"repository:a/b:pull\"",
                host(req)
            );
            return Response::new(401).header("WWW-Authenticate", &challenge);
        }
        manifest_response(&payload)
    });
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let r = reference(&server, "a/b:1.0");
    let first = client.fetch_manifest(&r).unwrap();
    assert_eq!(first.digest, Digest::of(&body));
    assert_eq!(
        first.manifest().unwrap().artifact_type.as_deref(),
        Some("application/vnd.slopvac.pack.v1")
    );
    client.fetch_manifest(&r).unwrap();
    assert_eq!(server.count("/token"), 1, "the cached token is reused");
    let token_req = server
        .requests()
        .into_iter()
        .find(|q| q.path.starts_with("/token"))
        .unwrap();
    assert!(token_req.path.contains("service=reg"));
    assert!(token_req.path.contains("scope=repository%3Aa%2Fb%3Apull"));
}

#[test]
fn tm35_a_token_realm_on_another_host_is_refused_and_no_credentials_are_sent() {
    for realm in [
        "http://localhost:{port}/token",
        "https://evil.example/token",
        "ftp://127.0.0.1/token",
    ] {
        let template = realm.to_owned();
        let server = Server::start(move |req| {
            let port = host(req).rsplit(':').next().unwrap().to_owned();
            Response::new(401).header(
                "WWW-Authenticate",
                &format!(
                    "Bearer realm=\"{}\",service=\"reg\"",
                    template.replace("{port}", &port)
                ),
            )
        });
        let client = client_with_basic("me", "s3cret");
        let err = client
            .fetch_manifest(&reference(&server, "a/b:1"))
            .unwrap_err();
        assert!(
            matches!(err, OciError::RealmRejected(_)),
            "{realm}: {err:?}"
        );
        assert!(!err.to_string().contains("s3cret"));
        for req in server.requests() {
            assert!(
                req.header("authorization").is_none(),
                "credentials reached the server: {req:?}"
            );
        }
    }
}

#[test]
fn tm36_authorization_is_dropped_on_a_cross_host_redirect() {
    let blob = b"layer bytes".to_vec();
    let cdn_body = blob.clone();
    let server = Server::start(move |req| {
        let port = host(req).rsplit(':').next().unwrap().to_owned();
        if req.path.starts_with("/cdn") {
            return Response::new(200).body(cdn_body.clone());
        }
        if req.header("authorization").is_none() {
            return Response::new(401).header("WWW-Authenticate", "Basic realm=\"reg\"");
        }
        Response::new(307).header("Location", &format!("http://localhost:{port}/cdn/blob"))
    });
    let desc = Descriptor::for_bytes("application/octet-stream", &blob);
    let mut out = Vec::new();
    client_with_basic("me", "pw")
        .pull_blob(&reference(&server, "a/b:1"), &desc, &mut out)
        .unwrap();
    assert_eq!(out, blob);
    let reqs = server.requests();
    let cdn = reqs.iter().find(|r| r.path.starts_with("/cdn")).unwrap();
    assert!(
        cdn.header("authorization").is_none(),
        "token leaked to the CDN host"
    );
    assert!(
        reqs.iter()
            .any(|r| r.path.starts_with("/v2/") && r.header("authorization").is_some())
    );
}

#[test]
fn tm36_redirects_to_plain_http_and_redirect_loops_are_refused() {
    let server = Server::start(|req| {
        if req.path.starts_with("/v2/a/b/blobs/") {
            return Response::new(302).header("Location", "http://example.com/blob");
        }
        Response::new(302).header("Location", &req.path)
    });
    let desc = Descriptor::for_bytes("x", b"x");
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let err = client
        .pull_blob(&reference(&server, "a/b:1"), &desc, &mut Vec::new())
        .unwrap_err();
    assert!(matches!(err, OciError::BadRedirect { .. }), "{err:?}");

    let looping = Server::start(|req| Response::new(302).header("Location", &req.path));
    let err = client
        .pull_blob(&reference(&looping, "a/b:1"), &desc, &mut Vec::new())
        .unwrap_err();
    assert!(matches!(err, OciError::BadRedirect { .. }), "{err:?}");
    assert_eq!(
        looping.requests().len(),
        6,
        "the first request plus 5 followed redirects"
    );
}

#[test]
fn tm33_a_blob_that_does_not_match_its_descriptor_fails() {
    let good = b"0123456789".to_vec();
    let desc = Descriptor::for_bytes("x", &good);
    let cases: [(&str, Vec<u8>); 3] = [
        ("wrong digest", b"9876543210".to_vec()),
        ("short", good[..5].to_vec()),
        ("long", [good.clone(), b"extra".to_vec()].concat()),
    ];
    for (name, body) in cases {
        let server = Server::start(move |_| Response::new(200).body(body.clone()));
        let mut out = Vec::new();
        let err = Client::with_loopback_http_for_tests(Box::new(Anonymous))
            .pull_blob(&reference(&server, "a/b:1"), &desc, &mut out)
            .unwrap_err();
        let ok = matches!(
            (&err, name),
            (OciError::DigestMismatch { .. }, "wrong digest")
                | (OciError::SizeMismatch { .. }, "short")
                | (
                    OciError::TooLarge { .. } | OciError::SizeMismatch { .. },
                    "long"
                )
        );
        assert!(ok, "{name}: {err:?}");
    }
}

#[test]
fn tm33_a_descriptor_above_256_mib_is_refused_before_any_request() {
    let server = Server::start(|_| Response::new(500));
    let mut desc = Descriptor::for_bytes("x", b"x");
    desc.size = 256 * 1024 * 1024 + 1;
    let err = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .pull_blob(&reference(&server, "a/b:1"), &desc, &mut Vec::new())
        .unwrap_err();
    assert!(matches!(err, OciError::TooLarge { .. }));
    assert!(server.requests().is_empty());
}

#[test]
fn tm34_oversized_manifests_and_unknown_media_types_are_refused() {
    let big = Server::start(|_| manifest_response(&vec![b' '; 5 * 1024 * 1024]));
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let err = client
        .fetch_manifest(&reference(&big, "a/b:1"))
        .unwrap_err();
    assert!(matches!(err, OciError::TooLarge { .. }), "{err:?}");

    let docker = Server::start(|_| {
        Response::new(200)
            .header(
                "Content-Type",
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .body(b"{\"schemaVersion\":2}".to_vec())
    });
    let err = client
        .fetch_manifest(&reference(&docker, "a/b:1"))
        .unwrap_err();
    assert!(matches!(err, OciError::UnsupportedMediaType(_)), "{err:?}");
}

#[test]
fn a_manifest_fetched_by_digest_must_hash_to_it() {
    let server = Server::start(|_| manifest_response(b"{\"schemaVersion\":2,\"mediaType\":\"x\"}"));
    let other = Digest::of(b"not it");
    let r = Reference::parse(&format!("{}/a/b@{other}", server.registry())).unwrap();
    let err = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .fetch_manifest(&r)
        .unwrap_err();
    assert!(matches!(err, OciError::DigestMismatch { .. }), "{err:?}");
}

#[test]
fn private_or_missing_packages_report_denied_not_a_crash() {
    let server = Server::start(|_| Response::new(403));
    let err = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert!(matches!(err, OciError::Denied(_)), "{err:?}");
}

#[test]
fn referrers_fall_back_to_the_tag_schema_and_filter_by_artifact_type() {
    let subject = Digest::of(b"subject");
    let sig = json!({"mediaType": MEDIA_MANIFEST, "digest": Digest::of(b"sig"), "size": 3,
        "artifactType": "application/vnd.slopvac.signature.v1"});
    let other = json!({"mediaType": MEDIA_MANIFEST, "digest": Digest::of(b"other"), "size": 5,
        "artifactType": "application/vnd.cncf.notary.signature"});
    let index = serde_json::to_vec(
        &json!({"schemaVersion": 2, "mediaType": MEDIA_INDEX, "manifests": [sig, other]}),
    )
    .unwrap();
    let tag_path = format!("/v2/a/b/manifests/sha256-{}", subject.hex());
    let server = Server::start(move |req| {
        if req.path.contains("/referrers/") {
            return Response::new(404);
        }
        if req.path == tag_path {
            return Response::new(200)
                .header("Content-Type", MEDIA_INDEX)
                .body(index.clone());
        }
        Response::new(404)
    });
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let r = reference(&server, "a/b:1");
    let all = client.referrers(&r, &subject, None).unwrap();
    assert_eq!(all.len(), 2);
    let sigs = client
        .referrers(&r, &subject, Some("application/vnd.slopvac.signature.v1"))
        .unwrap();
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].digest, Digest::of(b"sig"));
}

#[test]
fn referrers_with_neither_api_nor_tag_give_an_empty_list() {
    let server = Server::start(|_| Response::new(404));
    let r = reference(&server, "a/b:1");
    let found = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .referrers(&r, &Digest::of(b"s"), None)
        .unwrap();
    assert_eq!(found, Vec::<Descriptor>::new());
}

#[test]
fn tags_are_listed_across_pages() {
    let server = Server::start(|req| {
        if req.path.contains("last=") {
            return Response::new(200).json(&json!({"name": "a/b", "tags": ["1.1.0"]}));
        }
        Response::new(200)
            .header(
                "Link",
                "</v2/a/b/tags/list?n=1000&last=1.0.0>; rel=\"next\"",
            )
            .json(&json!({"name": "a/b", "tags": ["1.0.0"]}))
    });
    let tags = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .list_tags(&reference(&server, "a/b:1"))
        .unwrap();
    assert_eq!(tags, ["1.0.0", "1.1.0"]);
}

#[test]
fn push_then_pull_round_trips_blobs_and_a_manifest() {
    type Store = Arc<Mutex<HashMap<String, (String, Vec<u8>)>>>;
    let store: Store = Arc::default();
    let s = Arc::clone(&store);
    let server = Server::start(with_auth("reg", Some(("me", "pw")), move |req| {
        let mut s = s.lock().unwrap();
        let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
        match (req.method.as_str(), path) {
            ("HEAD", p) if p.contains("/blobs/sha256:") => {
                Response::new(if s.contains_key(p) { 200 } else { 404 })
            }
            ("POST", "/v2/a/b/blobs/uploads/") => {
                Response::new(202).header("Location", "/v2/a/b/blobs/uploads/u1")
            }
            ("PUT", "/v2/a/b/blobs/uploads/u1") => {
                let digest = query.strip_prefix("digest=").unwrap().to_owned();
                s.insert(
                    format!("/v2/a/b/blobs/{digest}"),
                    (String::new(), req.body.clone()),
                );
                Response::new(201)
            }
            ("PUT", p) if p.contains("/manifests/") => {
                s.insert(
                    p.to_owned(),
                    (
                        req.header("content-type").unwrap().to_owned(),
                        req.body.clone(),
                    ),
                );
                Response::new(201)
            }
            ("GET", p) if p.contains("/manifests/") => match s.get(p) {
                Some((ct, body)) => Response::new(200)
                    .header("Content-Type", ct)
                    .body(body.clone()),
                None => Response::new(404),
            },
            ("GET", p) if p.contains("/blobs/") => match s.get(p) {
                Some((_, body)) => Response::new(200).body(body.clone()),
                None => Response::new(404),
            },
            _ => Response::new(404),
        }
    }));
    let client = client_with_basic("me", "pw");
    let r = reference(&server, "a/b:1.0.0");
    let config = client
        .push_blob(&r, "application/vnd.oci.empty.v1+json", b"{}")
        .unwrap();
    let layer = client
        .push_blob(
            &r,
            "application/vnd.slopvac.pack.layer.v1.tar+gzip",
            b"payload",
        )
        .unwrap();
    let again = client
        .push_blob(
            &r,
            "application/vnd.slopvac.pack.layer.v1.tar+gzip",
            b"payload",
        )
        .unwrap();
    assert_eq!(again, layer);
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|q| q.method == "PUT" && q.path.starts_with("/v2/a/b/blobs/uploads/"))
            .count(),
        2,
        "an existing blob is not uploaded twice"
    );
    let body = manifest_json(&config, std::slice::from_ref(&layer));
    let digest = client.push_manifest(&r, MEDIA_MANIFEST, &body).unwrap();
    assert_eq!(digest, Digest::of(&body));

    let fetched = client.fetch_manifest(&r).unwrap();
    let manifest = fetched.manifest().unwrap();
    assert_eq!(manifest.layers, std::slice::from_ref(&layer));
    let mut out = Vec::new();
    client.pull_blob(&r, &manifest.layers[0], &mut out).unwrap();
    assert_eq!(out, b"payload");
    let scopes: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|q| q.path.starts_with("/token"))
        .map(|q| q.path)
        .collect();
    assert!(
        scopes
            .iter()
            .any(|p| p.contains("scope=repository%3Aa%2Fb%3Apull%2Cpush")),
        "publish asks for the push scope: {scopes:?}"
    );
}

#[test]
fn publish_without_credentials_is_refused_by_an_authenticating_registry() {
    let server = Server::start(with_auth("reg", Some(("me", "pw")), |_| Response::new(201)));
    let r = reference(&server, "a/b:1");
    for client in [
        Client::with_loopback_http_for_tests(Box::new(Anonymous)),
        client_with_basic("me", "wrong"),
    ] {
        let err = client.push_blob(&r, "x", b"data").unwrap_err();
        assert!(matches!(err, OciError::Denied(_)), "{err:?}");
    }
    assert!(
        server
            .requests()
            .iter()
            .all(|q| q.method != "POST" && q.method != "PUT"),
        "nothing was uploaded"
    );
}

#[test]
fn the_token_endpoint_receives_the_basic_credentials_and_the_pull_scope() {
    let body = manifest_json(&Descriptor::for_bytes("x", b"{}"), &[]);
    let server = Server::start(with_auth("reg", Some(("me", "s3cret")), move |_| {
        manifest_response(&body)
    }));
    client_with_basic("me", "s3cret")
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap();
    let token_req = server
        .requests()
        .into_iter()
        .find(|q| q.path.starts_with("/token"))
        .unwrap();
    assert_eq!(
        token_req.header("authorization"),
        Some(format!("Basic {}", base64_std(b"me:s3cret")).as_str())
    );
    assert!(token_req.path.contains("scope=repository%3Aa%2Fb%3Apull"));
    assert!(!token_req.path.contains("push"));
}

#[test]
fn wrong_credentials_at_the_token_endpoint_are_denied() {
    let server = Server::start(with_auth("reg", Some(("me", "s3cret")), |_| {
        Response::new(200)
    }));
    let err = client_with_basic("me", "nope")
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert!(matches!(err, OciError::Denied(_)), "{err:?}");
}

#[test]
fn tm35_a_cached_token_is_never_sent_to_another_registry() {
    let body = manifest_json(&Descriptor::for_bytes("x", b"{}"), &[]);
    let (b1, b2) = (body.clone(), body);
    let good = Server::start(with_auth("good", None, move |_| manifest_response(&b1)));
    let other = Server::start(with_auth("other", None, move |_| manifest_response(&b2)));
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    client.fetch_manifest(&reference(&good, "a/b:1")).unwrap();
    client.fetch_manifest(&reference(&other, "a/b:1")).unwrap();
    let first = other.requests().into_iter().next().unwrap();
    assert!(
        first.header("authorization").is_none(),
        "the first request to the second registry carried {:?}",
        first.header("authorization")
    );
    assert_eq!(
        other.count("/token"),
        1,
        "the second registry gets its own token"
    );
}

#[test]
fn tm35_a_realm_on_another_port_of_the_same_host_is_refused() {
    let evil = Server::start(|_| Response::new(200).json(&json!({"token": "x"})));
    let evil_port = evil.addr.port();
    let server = Server::start(move |_| {
        Response::new(401).header(
            "WWW-Authenticate",
            &format!("Bearer realm=\"http://127.0.0.1:{evil_port}/token\",service=\"reg\""),
        )
    });
    let err = client_with_basic("me", "s3cret")
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert!(matches!(err, OciError::RealmRejected(_)), "{err:?}");
    assert!(evil.requests().is_empty(), "the other port was contacted");
    assert!(
        server
            .requests()
            .iter()
            .all(|q| q.header("authorization").is_none())
    );
}

#[test]
fn tm36_the_production_client_never_speaks_plain_http() {
    let server = Server::start(|_| Response::new(200));
    let err = Client::with_credentials(Box::new(Anonymous))
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert!(matches!(err, OciError::Network { .. }), "{err:?}");
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn a_challenge_with_spaces_around_equals_signs_still_yields_a_clean_realm() {
    let body = manifest_json(&Descriptor::for_bytes("x", b"{}"), &[]);
    let server = Server::start(move |req| {
        if req.path.starts_with("/token") {
            return Response::new(200).json(&json!({"token": "T"}));
        }
        if req.header("authorization") == Some("Bearer T") {
            return manifest_response(&body);
        }
        let challenge = format!(
            "Bearer realm = \"http://{}/token\" , service = \"reg\"",
            host(req)
        );
        Response::new(401).header("WWW-Authenticate", &challenge)
    });
    Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap();
    assert_eq!(server.count("/token?service=reg"), 1);
}

#[test]
fn referrers_api_responses_are_used_and_paginated() {
    let subject = Digest::of(b"subject");
    let desc = |tag: &[u8]| {
        json!({"mediaType": MEDIA_MANIFEST, "digest": Digest::of(tag), "size": 1,
            "artifactType": "application/vnd.slopvac.signature.v1"})
    };
    let page = |m: serde_json::Value| {
        Response::new(200).header("Content-Type", MEDIA_INDEX).body(
            serde_json::to_vec(
                &json!({"schemaVersion": 2, "mediaType": MEDIA_INDEX, "manifests": [m]}),
            )
            .unwrap(),
        )
    };
    let (one, two) = (desc(b"one"), desc(b"two"));
    let server = Server::start(with_auth("reg", None, move |req| {
        if req.path.contains("page=2") {
            page(two.clone())
        } else if req.path.contains("/referrers/") {
            page(one.clone()).header("Link", "<?page=2>; rel=\"next\"")
        } else {
            Response::new(500)
        }
    }));
    let found = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .referrers(&reference(&server, "a/b:1"), &subject, None)
        .unwrap();
    let digests: Vec<_> = found.iter().map(|d| d.digest.clone()).collect();
    assert_eq!(digests, [Digest::of(b"one"), Digest::of(b"two")]);
    assert_eq!(server.count("/v2/a/b/manifests/"), 0, "no tag fallback");
}

#[test]
fn query_only_link_and_location_references_keep_the_path() {
    let server = Server::start(|req| {
        let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
        match (req.method.as_str(), path) {
            ("GET", "/v2/a/b/tags/list") if query.contains("last=") => {
                Response::new(200).json(&json!({"tags": ["1.1.0"]}))
            }
            ("GET", "/v2/a/b/tags/list") => Response::new(200)
                .header("Link", "<?n=1000&last=1.0.0>; rel=\"next\"")
                .json(&json!({"tags": ["1.0.0"]})),
            ("POST", "/v2/a/b/blobs/uploads/") => {
                Response::new(202).header("Location", "?_state=s1")
            }
            ("PUT", "/v2/a/b/blobs/uploads/") if query.starts_with("_state=s1&digest=") => {
                Response::new(201)
            }
            _ => Response::new(404),
        }
    });
    let client = Client::with_loopback_http_for_tests(Box::new(Anonymous));
    let r = reference(&server, "a/b:1");
    assert_eq!(client.list_tags(&r).unwrap(), ["1.0.0", "1.1.0"]);
    client.push_blob(&r, "x", b"data").unwrap();
}

#[test]
fn push_refuses_an_upload_location_on_another_host() {
    let server = Server::start(|req| {
        if req.method == "HEAD" {
            return Response::new(404);
        }
        Response::new(202).header("Location", "http://localhost:1/steal")
    });
    let err = Client::with_loopback_http_for_tests(Box::new(Anonymous))
        .push_blob(&reference(&server, "a/b:1"), "x", b"data")
        .unwrap_err();
    assert!(matches!(err, OciError::BadRedirect { .. }), "{err:?}");
}

#[test]
fn a_docker_config_auth_entry_becomes_basic_credentials() {
    let dir = std::env::temp_dir().join(format!("slopvac-oci-auth-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let server = Server::start(|req| match req.header("authorization") {
        Some(a) if a.starts_with("Basic ") => {
            manifest_response(b"{\"schemaVersion\":2,\"mediaType\":\"x\"}")
        }
        _ => Response::new(401).header("WWW-Authenticate", "Basic realm=\"reg\""),
    });
    let auth = base64_std(b"alice:pw1");
    let cfg = json!({"auths": {format!("https://{}/v1/", server.registry()): {"auth": auth}}});
    std::fs::write(dir.join("config.json"), cfg.to_string()).unwrap();
    let creds = DefaultCredentials {
        docker_config_dir: Some(dir.clone()),
        ..DefaultCredentials::default()
    };
    let r = Client::with_loopback_http_for_tests(Box::new(creds))
        .fetch_manifest(&reference(&server, "a/b:1"));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(r.is_ok(), "{r:?}");
    let sent = server
        .requests()
        .last()
        .unwrap()
        .header("authorization")
        .unwrap()
        .to_owned();
    assert_eq!(sent, format!("Basic {}", base64_std(b"alice:pw1")));
}

#[cfg(unix)]
mod helpers {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    fn setup(
        name: &str,
        store: &str,
        script: Option<&str>,
    ) -> (std::path::PathBuf, DefaultCredentials) {
        let dir =
            std::env::temp_dir().join(format!("slopvac-oci-helper-{name}-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            dir.join("config.json"),
            json!({"credsStore": store}).to_string(),
        )
        .unwrap();
        if let Some(body) = script {
            let p = bin.join(format!("docker-credential-{store}"));
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let creds = DefaultCredentials {
            docker_config_dir: Some(dir.clone()),
            helper_search_path: Some(bin.into_os_string()),
            helper_timeout: Some(Duration::from_secs(3)),
            ..DefaultCredentials::default()
        };
        (dir, creds)
    }

    fn lookup(
        name: &str,
        store: &str,
        script: Option<&str>,
    ) -> Result<Option<slopvac_plugins::oci::Credential>, OciError> {
        use slopvac_plugins::oci::CredentialSource;
        let (dir, creds) = setup(name, store, script);
        let r = creds.credential("ghcr.io");
        let _ = Path::new(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        r
    }

    #[test]
    fn a_helper_that_works_gives_credentials() {
        let got = lookup(
            "ok",
            "good",
            Some(
                r#"cat >/dev/null; echo '{"ServerURL":"ghcr.io","Username":"bob","Secret":"tok"}'"#,
            ),
        );
        let c = got.unwrap().unwrap();
        assert_eq!((c.username.as_str(), c.secret.as_str()), ("bob", "tok"));
    }

    #[test]
    fn tm38_helper_names_with_separators_or_shell_characters_are_refused() {
        for (i, bad) in ["../evil", "a/b", "x;y", "a b", "-flag"].iter().enumerate() {
            let err = lookup(&format!("bad{i}"), bad, None).unwrap_err();
            assert!(matches!(err, OciError::Credential(_)), "{bad}: {err:?}");
        }
    }

    #[test]
    fn tm38_a_helper_that_hangs_times_out() {
        let started = Instant::now();
        let err = lookup("hang", "slow", Some("sleep 30")).unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn tm38_a_helper_that_exits_but_leaves_a_descendant_holding_stdout_times_out() {
        let started = Instant::now();
        let err = lookup(
            "orphan",
            "leaky",
            Some(r#"cat >/dev/null; echo '{"Username":"u","Secret":"s"}'; sleep 6 &"#),
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Run `script` as the helper, with `$MARKER` naming a file. Returns the
    /// lookup result and the pid that the script's background job wrote there.
    fn lookup_with_descendant(name: &str, script: &str) -> (Result<(), OciError>, u32) {
        use slopvac_plugins::oci::CredentialSource;
        let (dir, creds) = setup(name, "tree", None);
        let marker = dir.join("descendant.pid");
        let bin = dir.join("bin").join("docker-credential-tree");
        let body = script.replace("$MARKER", marker.to_str().unwrap());
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let result = creds.credential("ghcr.io").map(drop);
        let pid = std::fs::read_to_string(&marker)
            .unwrap_or_default()
            .trim()
            .parse::<u32>()
            .expect("the descendant wrote its pid before the lookup ended");
        std::fs::remove_dir_all(&dir).unwrap();
        (result, pid)
    }

    fn process_is_gone(pid: u32) -> bool {
        let alive = || {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        // Killing is asynchronous: wait for it, with a generous limit.
        let deadline = Instant::now() + Duration::from_secs(20);
        while alive() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        true
    }

    const SPAWN_SLEEPER: &str = "sh -c 'echo $$ >\"$MARKER\"; exec sleep 300' &";

    #[test]
    fn tm38_a_timeout_kills_the_helpers_descendants() {
        let script = format!("cat >/dev/null\n{SPAWN_SLEEPER}\nsleep 300");
        let (result, pid) = lookup_with_descendant("tree-timeout", &script);
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(
            process_is_gone(pid),
            "descendant {pid} survived the timeout"
        );
    }

    #[test]
    fn tm38_a_timeout_after_the_helper_exited_kills_the_descendant_holding_stdout() {
        let script = format!("cat >/dev/null\n{SPAWN_SLEEPER}");
        let (result, pid) = lookup_with_descendant("tree-orphan", &script);
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(
            process_is_gone(pid),
            "descendant {pid} survived the timeout"
        );
    }

    #[test]
    fn tm38_an_output_flood_kills_the_descendant_that_writes_it() {
        let script = "cat >/dev/null\nsh -c 'echo $$ >\"$MARKER\"; exec yes' &\nsleep 300";
        let (result, pid) = lookup_with_descendant("tree-flood", script);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("printed more than")
        );
        assert!(process_is_gone(pid), "descendant {pid} survived the flood");
    }

    #[test]
    fn tm38_helper_output_over_64_kib_is_refused() {
        let err = lookup(
            "flood",
            "loud",
            Some("cat >/dev/null; head -c 10000000 /dev/zero"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("printed more than"), "{err}");
    }

    #[test]
    fn a_missing_helper_is_an_error_not_a_silent_anonymous_fallback() {
        let err = lookup("missing", "nothere", None).unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn a_helper_miss_falls_through_to_anonymous() {
        let got = lookup(
            "miss",
            "empty",
            Some("cat >/dev/null; echo 'credentials not found' >&2; exit 1"),
        );
        assert_eq!(got.unwrap(), None);
    }
}
