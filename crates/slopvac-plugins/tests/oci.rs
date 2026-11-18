//! OCI client tests against a scripted loopback registry (threat model section 6.4).

mod oci_server;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oci_server::{Request, Response, Server, base64_std, with_auth};
use serde_json::json;
use slopvac_plugins::oci::{
    Anonymous, Client, CredentialSource, DefaultCredentials, Descriptor, Digest, EnvCredentials,
    MEDIA_INDEX, MEDIA_MANIFEST, OciError, Reference,
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

// ---- pagination bounds, Link parsing, upload Locations and stalled uploads ----

fn tags_page(count: usize) -> Response {
    let tags: Vec<String> = (0..count).map(|i| format!("t{i}")).collect();
    Response::new(200).json(&json!({"name": "a/b", "tags": tags}))
}

fn referrers_page(count: usize) -> Response {
    let one = json!({"mediaType": MEDIA_MANIFEST, "digest": Digest::of(b"r"), "size": 1});
    let index = json!({"schemaVersion": 2, "mediaType": MEDIA_INDEX,
        "manifests": vec![one; count]});
    Response::new(200)
        .header("Content-Type", MEDIA_INDEX)
        .body(index.to_string())
}

fn anonymous_client() -> Client {
    Client::with_loopback_http_for_tests(Box::new(Anonymous))
}

#[test]
fn tag_listing_follows_at_most_ten_pages() {
    let server = Server::start(|_| tags_page(1).header("Link", "<?n=1000&page=2>; rel=\"next\""));
    let tags = anonymous_client()
        .list_tags(&reference(&server, "a/b:1"))
        .unwrap();
    assert_eq!(server.count("/v2/a/b/tags/list"), 10);
    assert_eq!(tags.len(), 10);
}

#[test]
fn tag_listing_accepts_exactly_ten_thousand_tags() {
    let server = Server::start(|_| tags_page(10_000));
    let tags = anonymous_client()
        .list_tags(&reference(&server, "a/b:1"))
        .unwrap();
    assert_eq!(tags.len(), 10_000);
}

#[test]
fn tag_listing_rejects_ten_thousand_and_one_tags() {
    let server = Server::start(|_| tags_page(10_001));
    let err = anonymous_client()
        .list_tags(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert!(
        matches!(err, OciError::TooLarge { limit: 10_000, .. }),
        "{err:?}"
    );
}

#[test]
fn referrers_follow_at_most_ten_pages() {
    let server = Server::start(|_| referrers_page(1).header("Link", "<?page=2>; rel=\"next\""));
    let found = anonymous_client()
        .referrers(&reference(&server, "a/b:1"), &Digest::of(b"s"), None)
        .unwrap();
    assert_eq!(server.count("/v2/a/b/referrers/"), 10);
    assert_eq!(found.len(), 10);
}

#[test]
fn referrers_accept_exactly_ten_thousand_descriptors() {
    let server = Server::start(|_| referrers_page(10_000));
    let found = anonymous_client()
        .referrers(&reference(&server, "a/b:1"), &Digest::of(b"s"), None)
        .unwrap();
    assert_eq!(found.len(), 10_000);
}

#[test]
fn referrers_reject_ten_thousand_and_one_descriptors() {
    let server = Server::start(|_| referrers_page(10_001));
    let err = anonymous_client()
        .referrers(&reference(&server, "a/b:1"), &Digest::of(b"s"), None)
        .unwrap_err();
    assert!(
        matches!(err, OciError::TooLarge { limit: 10_000, .. }),
        "{err:?}"
    );
}

/// A registry without the referrers API whose `sha256-<hex>` tag holds `count` descriptors.
fn tag_schema_server(subject: &Digest, count: usize) -> Server {
    let one = json!({"mediaType": MEDIA_MANIFEST, "digest": Digest::of(b"r"), "size": 1});
    let index = json!({"schemaVersion": 2, "mediaType": MEDIA_INDEX,
        "manifests": vec![one; count]})
    .to_string();
    let tag_path = format!("/v2/a/b/manifests/sha256-{}", subject.hex());
    Server::start(move |req| {
        if req.path == tag_path {
            Response::new(200)
                .header("Content-Type", MEDIA_INDEX)
                .body(index.clone())
        } else {
            Response::new(404)
        }
    })
}

#[test]
fn the_tag_schema_fallback_accepts_exactly_ten_thousand_descriptors() {
    let subject = Digest::of(b"s");
    let server = tag_schema_server(&subject, 10_000);
    let found = anonymous_client()
        .referrers(&reference(&server, "a/b:1"), &subject, None)
        .unwrap();
    assert_eq!(found.len(), 10_000);
}

#[test]
fn the_tag_schema_fallback_rejects_ten_thousand_and_one_descriptors() {
    let subject = Digest::of(b"s");
    let server = tag_schema_server(&subject, 10_001);
    let err = anonymous_client()
        .referrers(&reference(&server, "a/b:1"), &subject, None)
        .unwrap_err();
    assert!(
        matches!(err, OciError::TooLarge { limit: 10_000, .. }),
        "{err:?}"
    );
}

#[test]
fn the_tag_schema_fallback_gate_counts_descriptors_before_the_type_filter() {
    let subject = Digest::of(b"s");
    let server = tag_schema_server(&subject, 10_001);
    let err = anonymous_client()
        .referrers(
            &reference(&server, "a/b:1"),
            &subject,
            Some("application/vnd.slopvac.signature.v1"),
        )
        .unwrap_err();
    assert!(matches!(err, OciError::TooLarge { .. }), "{err:?}");
}

#[test]
fn referrers_api_responses_with_an_unsupported_content_type_are_refused() {
    for content_type in ["text/html", MEDIA_MANIFEST] {
        let index = json!({"schemaVersion": 2, "mediaType": MEDIA_INDEX, "manifests": []});
        let server = Server::start(move |_| {
            Response::new(200)
                .header("Content-Type", content_type)
                .body(index.to_string())
        });
        let err = anonymous_client()
            .referrers(&reference(&server, "a/b:1"), &Digest::of(b"s"), None)
            .unwrap_err();
        assert!(
            matches!(&err, OciError::UnsupportedMediaType(t) if t == content_type),
            "{content_type}: {err:?}"
        );
    }
}

#[test]
fn the_next_page_is_the_link_with_rel_next_wherever_it_appears() {
    // Each case is the `Link` header lines of the first page; "b" is on page 2.
    let cases: [(&[&str], &[&str]); 8] = [
        (
            &["<?page=0>; rel=\"prev\", <?page=2>; rel=\"next\""],
            &["a", "b"],
        ),
        (&["<?page=0>; rel=\"prev\""], &["a"]),
        (
            &["<?page=0>; rel=\"prev\"", "<?page=2>; rel=\"next\""],
            &["a", "b"],
        ),
        (&["<?page=2>; rel=next"], &["a", "b"]),
        (&["<?page=2>; title=\"x\"; rel=\"Last NEXT\""], &["a", "b"]),
        (
            &["<?page=0>; title=\"a, <?page=2>; rel=next\"; rel=\"prev\""],
            &["a"],
        ),
        (&["<?page=2>"], &["a"]),
        // An escaped quote inside a quoted parameter does not end it.
        (&[r#"<?page=2>; title="a\"b"; rel="next""#], &["a", "b"]),
    ];
    for (links, want) in cases {
        let server = Server::start(move |req| {
            if req.path.contains("page=2") {
                return Response::new(200).json(&json!({"tags": ["b"]}));
            }
            if req.path.contains("page=0") {
                return Response::new(500);
            }
            links
                .iter()
                .fold(Response::new(200), |r, l| r.header("Link", l))
                .json(&json!({"tags": ["a"]}))
        });
        let got = anonymous_client()
            .list_tags(&reference(&server, "a/b:1"))
            .unwrap();
        assert_eq!(got, want, "{links:?}");
    }
}

#[test]
fn the_upload_digest_is_not_appended_after_a_location_fragment() {
    // Absolute and network-path Locations that carry a fragment.
    for form in ["absolute", "network-path"] {
        let server = Server::start(move |req| {
            let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
            match (req.method.as_str(), path) {
                ("HEAD", _) => Response::new(404),
                ("POST", _) => {
                    let h = host(req);
                    let location = match form {
                        "absolute" => format!("http://{h}/v2/a/b/blobs/uploads/u1?_state=s#frag"),
                        _ => format!("//{h}/v2/a/b/blobs/uploads/u1?_state=s#frag"),
                    };
                    Response::new(202).header("Location", &location)
                }
                ("PUT", "/v2/a/b/blobs/uploads/u1")
                    if query.starts_with("_state=s&digest=sha256:") =>
                {
                    Response::new(201)
                }
                _ => Response::new(400),
            }
        });
        let pushed = anonymous_client().push_blob(&reference(&server, "a/b:1"), "x", b"data");
        assert!(pushed.is_ok(), "{form}: {pushed:?} {:?}", server.requests());
    }
}

#[test]
fn an_upload_to_a_registry_that_stops_reading_fails_instead_of_blocking() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut out = conn;
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                let reply = match first.split(' ').next() {
                    Some("HEAD") => "HTTP/1.1 404 X\r\nContent-Length: 0\r\n\r\n".to_owned(),
                    Some("POST") => format!(
                        "HTTP/1.1 202 X\r\nLocation: http://127.0.0.1:{port}/v2/a/b/blobs/uploads/u\r\nContent-Length: 0\r\n\r\n"
                    ),
                    // PUT: accept the connection and never read the body.
                    _ => {
                        std::thread::sleep(Duration::from_secs(120));
                        return;
                    }
                };
                let _ = out.write_all(reply.as_bytes());
            });
        }
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let client =
            Client::with_send_timeout_for_tests(Box::new(Anonymous), Duration::from_secs(1));
        let r = Reference::parse(&format!("127.0.0.1:{port}/a/b:1")).unwrap();
        // Larger than any socket buffer pair, so the sender must block.
        let _ = tx.send(client.push_blob(&r, "x", &vec![0u8; 64 << 20]));
    });
    let outcome = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("push_blob blocked on a registry that stopped reading");
    assert!(
        matches!(outcome, Err(OciError::Network { .. })),
        "{outcome:?}"
    );
}
// ---- size gates: caller maximum, blob and manifest uploads ----

#[test]
fn fetch_blob_bytes_accepts_a_blob_of_exactly_the_caller_maximum() {
    let data = b"four";
    let desc = Descriptor::for_bytes("x", data);
    let server = Server::start(move |_| Response::new(200).body(data.to_vec()));
    let got = anonymous_client()
        .fetch_blob_bytes(&reference(&server, "a/b:1"), &desc, 4)
        .unwrap();
    assert_eq!(got, data);
}

#[test]
fn fetch_blob_bytes_refuses_one_byte_over_the_caller_maximum_before_any_request() {
    let desc = Descriptor::for_bytes("x", b"fives");
    let server = Server::start(|_| Response::new(500));
    let err = anonymous_client()
        .fetch_blob_bytes(&reference(&server, "a/b:1"), &desc, 4)
        .unwrap_err();
    assert!(
        matches!(err, OciError::TooLarge { limit: 4, .. }),
        "{err:?}"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn push_blob_refuses_one_byte_over_256_mib_before_any_request() {
    let server = Server::start(|_| Response::new(500));
    let bytes = vec![0u8; (256 << 20) + 1];
    let err = anonymous_client()
        .push_blob(&reference(&server, "a/b:1"), "x", &bytes)
        .unwrap_err();
    assert!(matches!(err, OciError::TooLarge { .. }), "{err:?}");
    assert!(server.requests().is_empty());
}

#[test]
fn push_manifest_accepts_exactly_four_mib() {
    let server = Server::start(|_| Response::new(201));
    let body = vec![b' '; 4 << 20];
    let digest = anonymous_client()
        .push_manifest(&reference(&server, "a/b:1"), MEDIA_MANIFEST, &body)
        .unwrap();
    assert_eq!(digest, Digest::of(&body));
}

#[test]
fn push_manifest_refuses_one_byte_over_four_mib_before_any_request() {
    let server = Server::start(|_| Response::new(500));
    let body = vec![b' '; (4 << 20) + 1];
    let err = anonymous_client()
        .push_manifest(&reference(&server, "a/b:1"), MEDIA_MANIFEST, &body)
        .unwrap_err();
    assert!(matches!(err, OciError::TooLarge { .. }), "{err:?}");
    assert!(server.requests().is_empty());
}

// ---- environment discovery through the public constructor ----

#[test]
fn docker_config_dir_discovery_follows_each_platforms_home() {
    use std::path::{Path, PathBuf};
    for (windows, pairs, want) in [
        (
            false,
            vec![("HOME", "/h")],
            Some(Path::new("/h").join(".docker")),
        ),
        (false, vec![("USERPROFILE", "C:\\u")], None),
        (
            true,
            vec![("USERPROFILE", "C:\\u")],
            Some(Path::new("C:\\u").join(".docker")),
        ),
        (
            true,
            vec![("HOME", "/h")],
            Some(Path::new("/h").join(".docker")),
        ),
        (
            true,
            vec![("USERPROFILE", "C:\\u"), ("HOME", "/h")],
            Some(Path::new("C:\\u").join(".docker")),
        ),
        (
            true,
            vec![("DOCKER_CONFIG", "/d"), ("USERPROFILE", "C:\\u")],
            Some(PathBuf::from("/d")),
        ),
        (false, vec![], None),
    ] {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let got = DefaultCredentials::from_lookup_for_tests(|k| map.get(k).cloned(), windows)
            .docker_config_dir;
        assert_eq!(got, want, "windows={windows} {pairs:?}");
    }
}

/// Windows runs only `.exe` and `.com` helpers. The files here are empty, so a
/// found helper fails to start; "not found" means discovery skipped it.
#[cfg(windows)]
mod windows_helpers {
    use super::*;
    use slopvac_plugins::oci::CredentialSource;

    fn lookup_with_file(tag: &str, store: &str, file: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("slopvac-oci-win-{tag}-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            dir.join("config.json"),
            json!({"credsStore": store}).to_string(),
        )
        .unwrap();
        std::fs::write(bin.join(file), b"").unwrap();
        let creds = DefaultCredentials {
            docker_config_dir: Some(dir.clone()),
            helper_search_path: Some(bin.into_os_string()),
            project_root: Some(dir.join("project")),
            ..DefaultCredentials::default()
        };
        let msg = creds.credential("ghcr.io").unwrap_err().to_string();
        std::fs::remove_dir_all(&dir).unwrap();
        msg
    }

    #[test]
    fn a_bare_helper_name_finds_its_exe() {
        let msg = lookup_with_file("bare", "pass", "docker-credential-pass.exe");
        assert!(msg.contains("cannot start"), "{msg}");
    }

    #[test]
    fn a_helper_name_with_its_suffix_is_found_as_written() {
        let msg = lookup_with_file("suffixed", "desktop.exe", "docker-credential-desktop.exe");
        assert!(msg.contains("cannot start"), "{msg}");
    }

    #[test]
    fn a_batch_file_is_never_a_helper() {
        let msg = lookup_with_file("bat", "x", "docker-credential-x.bat");
        assert!(msg.contains("not found"), "{msg}");
    }

    /// Build a tiny executable from `source` with the toolchain that runs the tests.
    fn compile(source: &str, out: &std::path::Path) {
        let src = out.with_extension("rs");
        std::fs::write(&src, source).unwrap();
        let status = std::process::Command::new("rustc")
            .arg("--edition=2021")
            .arg("-o")
            .arg(out)
            .arg(&src)
            .status()
            .unwrap();
        assert!(status.success(), "rustc failed for {}", out.display());
    }

    /// Removes the decoy from the application directory, even when the test fails.
    struct RemoveOnDrop(std::path::PathBuf);

    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn pid_is_gone(pid: u32) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let listing = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .unwrap();
            if !String::from_utf8_lossy(&listing.stdout).contains(&pid.to_string()) {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Windows resolves a bare `taskkill` from the application directory before
    /// System32, so a decoy there would run if the kill used the bare name.
    #[test]
    fn tm38_a_timeout_runs_system32_taskkill_never_a_decoy_found_by_name() {
        let dir = std::env::temp_dir().join(format!("slopvac-oci-win-kill-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            dir.join("config.json"),
            json!({"credsStore": "hang"}).to_string(),
        )
        .unwrap();
        let pid_file = dir.join("helper.pid");
        let decoy_marker = dir.join("decoy-ran");
        compile(
            &format!(
                "fn main() {{ std::fs::write({pid_file:?}, std::process::id().to_string()).unwrap(); \
                 std::thread::sleep(std::time::Duration::from_secs(600)); }}"
            ),
            &bin.join("docker-credential-hang.exe"),
        );
        let app_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        let decoy = app_dir.join("taskkill.exe");
        let _cleanup = RemoveOnDrop(decoy.clone());
        let built = dir.join("taskkill.exe");
        compile(
            &format!("fn main() {{ std::fs::write({decoy_marker:?}, \"ran\").unwrap(); }}"),
            &built,
        );
        std::fs::copy(&built, &decoy).unwrap();

        let creds = DefaultCredentials {
            docker_config_dir: Some(dir.clone()),
            helper_search_path: Some(bin.clone().into_os_string()),
            project_root: Some(dir.join("project")),
            helper_timeout: Some(std::time::Duration::from_secs(5)),
            ..DefaultCredentials::default()
        };
        let err = creds.credential("ghcr.io").unwrap_err().to_string();

        assert!(err.contains("timed out"), "{err}");
        assert!(!decoy_marker.exists(), "the decoy taskkill.exe ran");
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .expect("the helper wrote its pid before the lookup ended")
            .trim()
            .parse()
            .unwrap();
        assert!(pid_is_gone(pid), "helper {pid} survived the timeout");
        std::fs::remove_dir_all(&dir).unwrap();
    }
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
            project_root: Some(dir.join("project")),
            helper_timeout: Some(Duration::from_secs(8)),
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
    fn a_helper_configured_with_its_executable_suffix_is_found() {
        let got = lookup(
            "suffixed",
            "desktop.exe",
            Some(r#"cat >/dev/null; echo '{"Username":"bob","Secret":"tok"}'"#),
        );
        assert_eq!(got.unwrap().unwrap().username, "bob");
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
        let err = lookup("hang", "slow", Some("sleep 120")).unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
    }

    #[test]
    fn tm38_a_helper_that_exits_but_leaves_a_descendant_holding_stdout_times_out() {
        let err = lookup(
            "orphan",
            "leaky",
            Some(r#"cat >/dev/null; echo '{"Username":"u","Secret":"s"}'; sleep 60 &"#),
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
    }

    /// Run `script` as the helper, with `$MARKER` naming a file. Returns the
    /// lookup result and the pid that the script's background job wrote there.
    ///
    /// The helper's deadline starts at spawn, so on a loaded machine it can fire before
    /// the background job has written its pid. That attempt proves nothing about the
    /// kill, so retry with a doubled deadline until the pid is on disk.
    fn lookup_with_descendant(name: &str, script: &str) -> (Result<(), OciError>, u32) {
        use slopvac_plugins::oci::CredentialSource;
        let (dir, mut creds) = setup(name, "tree", None);
        let marker = dir.join("descendant.pid");
        let bin = dir.join("bin").join("docker-credential-tree");
        let body = script.replace("$MARKER", marker.to_str().unwrap());
        std::fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut timeout = Duration::from_secs(8);
        for _ in 0..4 {
            creds.helper_timeout = Some(timeout);
            let result = creds.credential("ghcr.io").map(drop);
            if let Some(pid) = std::fs::read_to_string(&marker)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok())
            {
                std::fs::remove_dir_all(&dir).unwrap();
                return (result, pid);
            }
            timeout *= 2;
        }
        panic!("the descendant never wrote its pid before any of the deadlines");
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

    /// A helper that records that it ran, then answers like a good one.
    fn marking_script(marker: &Path) -> String {
        format!(
            "#!/bin/sh\ntouch '{}'\ncat >/dev/null\necho '{{\"Username\":\"bob\",\"Secret\":\"tok\"}}'\n",
            marker.display()
        )
    }

    fn plant(dir: &Path, store: &str, marker: &Path, mode: u32) {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join(format!("docker-credential-{store}"));
        std::fs::write(&file, marking_script(marker)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Credentials whose helper is looked up through `search`, with a project
    /// at `project`.
    fn creds_for(scratch: &Path, store: &str, search: &str, project: &Path) -> DefaultCredentials {
        std::fs::write(
            scratch.join("config.json"),
            json!({"credsStore": store}).to_string(),
        )
        .unwrap();
        DefaultCredentials {
            docker_config_dir: Some(scratch.to_path_buf()),
            helper_search_path: Some(search.into()),
            project_root: Some(project.to_path_buf()),
            helper_timeout: Some(Duration::from_secs(8)),
            ..DefaultCredentials::default()
        }
    }

    fn scratch() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        (dir, base)
    }

    /// The lookup is refused with the typed error and the helper never ran.
    fn refused(creds: &DefaultCredentials, marker: &Path) -> OciError {
        use slopvac_plugins::oci::CredentialSource;
        let err = creds.credential("ghcr.io").unwrap_err();
        assert!(matches!(err, OciError::HelperUntrusted { .. }), "{err:?}");
        assert!(!marker.exists(), "the planted helper ran");
        err
    }

    #[test]
    fn tm38_a_helper_in_a_relative_path_entry_is_not_run() {
        let (_guard, base) = scratch();
        let marker = base.join("ran");
        plant(&base.join("rel"), "planted", &marker, 0o755);
        // The same directory, written relative to the working directory.
        let cwd = std::env::current_dir().unwrap();
        let up = "../".repeat(cwd.components().count() - 1);
        let relative = format!(
            "{up}{}",
            base.join("rel").strip_prefix("/").unwrap().display()
        );
        let creds = creds_for(&base, "planted", &relative, &base.join("project"));
        refused(&creds, &marker);
    }

    #[test]
    fn tm38_a_helper_in_an_empty_path_entry_is_not_run() {
        // An empty entry means the working directory.
        let (_guard, base) = scratch();
        let marker = base.join("ran");
        let store = format!("planted{}", std::process::id());
        let cwd = std::env::current_dir().unwrap();
        let file = cwd.join(format!("docker-credential-{store}"));
        std::fs::write(&file, marking_script(&marker)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let creds = creds_for(&base, &store, "", &base.join("project"));
        let result = std::panic::catch_unwind(|| refused(&creds, &marker));
        std::fs::remove_file(&file).unwrap();
        result.unwrap();
    }

    /// `dir` is relative to the scratch base, whose `project` subdirectory is
    /// the project. `shown` is how the refusal names the directory (policies
    /// section 1.7: project-relative, or `<external>/<name>` outside it).
    #[rstest::rstest]
    #[case::project_local_directory("project/tools", 0o755, "tools")]
    #[case::group_writable_directory("shared", 0o775, "<external>/shared")]
    #[case::world_writable_directory("open", 0o777, "<external>/open")]
    fn tm38_a_helper_in_an_untrusted_directory_is_not_run(
        #[case] dir: &str,
        #[case] mode: u32,
        #[case] shown: &str,
    ) {
        let (_guard, base) = scratch();
        let marker = base.join("ran");
        let bin = base.join(dir);
        plant(&bin, "planted", &marker, mode);
        let creds = creds_for(
            &base,
            "planted",
            bin.to_str().unwrap(),
            &base.join("project"),
        );
        let err = refused(&creds, &marker);
        assert_eq!(err.code(), Some("credential-helper-untrusted"));
        assert_eq!(
            err.to_string(),
            format!(
                "credential helper `docker-credential-planted` was not found in a trusted \
                 directory; it exists only in {shown}. Tighten the permissions of that \
                 directory so that only you or root can write it, or sign in with a static \
                 `auths` login in the Docker config, with SLOPVAC_OCI_USER and \
                 SLOPVAC_OCI_TOKEN, or with GITHUB_TOKEN for ghcr.io"
            )
        );
    }

    #[test]
    fn tm38_an_auth_file_login_wins_over_a_configured_helper_and_no_helper_starts() {
        use slopvac_plugins::oci::CredentialSource;
        let (_guard, base) = scratch();
        let marker = base.join("ran");
        plant(&base.join("trusted"), "planted", &marker, 0o755);
        let mut creds = creds_for(
            &base,
            "planted",
            base.join("trusted").to_str().unwrap(),
            &base.join("project"),
        );
        let auth = base64_std(b"from-auth-file:pw");
        let file = base.join("auth.json");
        std::fs::write(
            &file,
            json!({"auths": {"ghcr.io": {"auth": auth}}}).to_string(),
        )
        .unwrap();
        creds.auth_files = vec![file];
        let got = creds.credential("ghcr.io").unwrap().unwrap();
        assert_eq!(got.username, "from-auth-file");
        assert!(!marker.exists(), "the helper started");
    }

    #[test]
    fn tm38_without_a_project_root_no_helper_runs() {
        use slopvac_plugins::oci::CredentialSource;
        // A trusted directory, so only the unknown project stops the helper.
        let (_guard, base) = scratch();
        let marker = base.join("ran");
        plant(&base.join("trusted"), "planted", &marker, 0o755);
        let mut creds = creds_for(
            &base,
            "planted",
            base.join("trusted").to_str().unwrap(),
            &base,
        );
        creds.project_root = None;
        let err = creds.credential("ghcr.io").unwrap_err();
        assert_eq!(err.code(), Some("credential-helper-project-unknown"));
        assert_eq!(
            err.to_string(),
            "credential helper `docker-credential-planted` was not run: the project \
             directory is unknown"
        );
        assert!(!marker.exists(), "the helper ran");
    }

    #[test]
    fn tm38_from_env_keeps_helpers_out_of_a_checkout_the_working_directory_is_inside() {
        // The working directory may be a subdirectory of the checkout, so the
        // project root comes from the caller and a sibling `tools/` inside the
        // checkout is still inside the project.
        let (_guard, base) = scratch();
        let marker = base.join("ran");
        let checkout = base.join("checkout");
        let tools = checkout.join("tools");
        plant(&tools, "planted", &marker, 0o755);
        let mut creds = DefaultCredentials::from_env(&checkout);
        creds.docker_config_dir = Some(base.clone());
        creds.helper_search_path = Some(tools.into_os_string());
        std::fs::write(
            base.join("config.json"),
            json!({"credsStore": "planted"}).to_string(),
        )
        .unwrap();
        refused(&creds, &marker);
    }

    #[test]
    fn tm38_an_untrusted_entry_does_not_hide_the_helper_in_a_trusted_one() {
        use slopvac_plugins::oci::CredentialSource;
        let (_guard, base) = scratch();
        let (bad_marker, good_marker) = (base.join("bad"), base.join("good"));
        let project = base.join("project");
        plant(&project.join("tools"), "both", &bad_marker, 0o755);
        plant(&base.join("trusted"), "both", &good_marker, 0o755);
        let search = std::env::join_paths([project.join("tools"), base.join("trusted")]).unwrap();
        let creds = creds_for(&base, "both", search.to_str().unwrap(), &project);
        let got = creds.credential("ghcr.io").unwrap().unwrap();
        assert_eq!(got.username, "bob");
        assert!(good_marker.exists(), "the trusted helper did not run");
        assert!(!bad_marker.exists(), "the planted helper ran");
    }
}

#[test]
fn github_token_applies_to_ghcr_whatever_the_host_case() {
    let creds = DefaultCredentials {
        env: EnvCredentials {
            github_token: Some("t".into()),
            ..EnvCredentials::default()
        },
        ..DefaultCredentials::default()
    };
    for host in ["ghcr.io", "GHCR.IO", "Ghcr.Io"] {
        let c = creds.credential(host).unwrap();
        assert_eq!(c.map(|c| c.secret), Some("t".to_owned()), "{host}");
    }
    assert!(creds.credential("ghcr.io.evil.example").unwrap().is_none());
}

#[test]
fn docker_config_auth_keys_match_the_host_case_insensitively_and_the_port_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let auth = base64_std(b"alice:pw1");
    let cfg = json!({"auths": {
        "https://Registry.Example:5000/v1/": {"auth": auth},
    }});
    std::fs::write(dir.path().join("config.json"), cfg.to_string()).unwrap();
    let creds = DefaultCredentials {
        docker_config_dir: Some(dir.path().to_path_buf()),
        ..DefaultCredentials::default()
    };
    for host in ["registry.example:5000", "REGISTRY.EXAMPLE:5000"] {
        let c = creds.credential(host).unwrap();
        assert_eq!(c.map(|c| c.username), Some("alice".to_owned()), "{host}");
    }
    for other in ["registry.example:5001", "registry.example"] {
        assert!(creds.credential(other).unwrap().is_none(), "{other}");
    }
}

/// Login order after the environment: the Docker config's `auths`, then the
/// first registry auth file, then the helper the Docker config names.
mod login_order {
    use super::*;

    fn write_config(dir: &std::path::Path, name: &str, user: &str) -> std::path::PathBuf {
        let file = dir.join(name);
        let auth = base64_std(format!("{user}:pw").as_bytes());
        let cfg = json!({"auths": {"registry.example": {"auth": auth}}});
        std::fs::write(&file, cfg.to_string()).unwrap();
        file
    }

    fn docker_dir_with(config: &serde_json::Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), config.to_string()).unwrap();
        dir
    }

    fn user_of(creds: &DefaultCredentials) -> Option<String> {
        creds
            .credential("registry.example")
            .unwrap()
            .map(|c| c.username)
    }

    #[test]
    fn a_registry_auth_file_login_is_used_when_the_docker_config_has_none() {
        let docker = docker_dir_with(&json!({}));
        let other = tempfile::tempdir().unwrap();
        let creds = DefaultCredentials {
            docker_config_dir: Some(docker.path().to_path_buf()),
            auth_files: vec![write_config(other.path(), "auth.json", "from-auth-file")],
            ..DefaultCredentials::default()
        };
        assert_eq!(user_of(&creds).as_deref(), Some("from-auth-file"));
    }

    #[test]
    fn the_docker_config_beats_a_registry_auth_file() {
        let docker = tempfile::tempdir().unwrap();
        write_config(docker.path(), "config.json", "from-docker");
        let other = tempfile::tempdir().unwrap();
        let creds = DefaultCredentials {
            docker_config_dir: Some(docker.path().to_path_buf()),
            auth_files: vec![write_config(other.path(), "auth.json", "from-auth-file")],
            ..DefaultCredentials::default()
        };
        assert_eq!(user_of(&creds).as_deref(), Some("from-docker"));
    }

    #[test]
    fn only_the_first_auth_file_that_exists_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let creds = DefaultCredentials {
            auth_files: vec![
                dir.path().join("missing.json"),
                write_config(dir.path(), "first.json", "first"),
                write_config(dir.path(), "second.json", "second"),
            ],
            ..DefaultCredentials::default()
        };
        assert_eq!(user_of(&creds).as_deref(), Some("first"));
    }

    #[test]
    fn a_static_login_is_used_without_starting_the_helper_the_config_names() {
        // The helper does not exist and no project root is set: running it
        // would fail the lookup.
        let docker = tempfile::tempdir().unwrap();
        let auth = base64_std(b"static:pw");
        std::fs::write(
            docker.path().join("config.json"),
            json!({
                "auths": {"registry.example": {"auth": auth}},
                "credsStore": "absent",
            })
            .to_string(),
        )
        .unwrap();
        let creds = DefaultCredentials {
            docker_config_dir: Some(docker.path().to_path_buf()),
            ..DefaultCredentials::default()
        };
        assert_eq!(user_of(&creds).as_deref(), Some("static"));
    }

    #[test]
    fn environment_credentials_beat_every_file() {
        let docker = tempfile::tempdir().unwrap();
        write_config(docker.path(), "config.json", "from-docker");
        let creds = DefaultCredentials {
            env: EnvCredentials {
                user: Some("env-user".into()),
                token: Some("t".into()),
                ..EnvCredentials::default()
            },
            docker_config_dir: Some(docker.path().to_path_buf()),
            ..DefaultCredentials::default()
        };
        assert_eq!(user_of(&creds).as_deref(), Some("env-user"));
    }

    #[rstest::rstest]
    #[case::registry_auth_file_first(
        &[("REGISTRY_AUTH_FILE", "/r/auth.json"), ("XDG_RUNTIME_DIR", "/run/u"), ("XDG_CONFIG_HOME", "/c"), ("HOME", "/h")],
        &["/r/auth.json", "/run/u/containers/auth.json", "/c/containers/auth.json"]
    )]
    #[case::config_home_falls_back_to_dot_config(
        &[("HOME", "/h")],
        &["/h/.config/containers/auth.json"]
    )]
    #[case::nothing_set(&[], &[])]
    fn auth_file_candidates_follow_the_container_tools_order(
        #[case] vars: &[(&str, &str)],
        #[case] want: &[&str],
    ) {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let got =
            DefaultCredentials::from_lookup_for_tests(|k| map.get(k).cloned(), false).auth_files;
        let want: Vec<std::path::PathBuf> = want.iter().map(std::path::PathBuf::from).collect();
        assert_eq!(got, want);
    }
}

/// A credential source whose helper was refused.
struct Refusing(OciError);

impl CredentialSource for Refusing {
    fn credential(&self, _: &str) -> Result<Option<slopvac_plugins::oci::Credential>, OciError> {
        Err(self.0.clone())
    }
}

fn untrusted() -> OciError {
    OciError::HelperUntrusted {
        program: "docker-credential-x".into(),
        dir: "<external>/bin".into(),
    }
}

fn project_unknown() -> OciError {
    OciError::HelperProjectUnknown("docker-credential-x".into())
}

#[rstest::rstest]
#[case::untrusted_helper(untrusted())]
#[case::unknown_project(project_unknown())]
fn a_refused_helper_does_not_stop_a_registry_that_serves_anonymous_tokens(
    #[case] refusal: OciError,
) {
    let body = manifest_json(&Descriptor::for_bytes("x", b"{}"), &[]);
    let server = Server::start(with_auth("reg", None, move |_| manifest_response(&body)));
    let client = Client::with_loopback_http_for_tests(Box::new(Refusing(refusal)));
    client.fetch_manifest(&reference(&server, "a/b:1")).unwrap();
    let token_req = server
        .requests()
        .into_iter()
        .find(|q| q.path.starts_with("/token"))
        .unwrap();
    assert!(token_req.header("authorization").is_none());
}

#[rstest::rstest]
#[case::untrusted_helper(untrusted(), "credential-helper-untrusted")]
#[case::unknown_project(project_unknown(), "credential-helper-project-unknown")]
fn a_refused_helper_is_reported_when_the_token_endpoint_needs_a_login(
    #[case] refusal: OciError,
    #[case] code: &str,
) {
    let server = Server::start(with_auth("reg", Some(("me", "s3cret")), |_| {
        Response::new(200)
    }));
    let client = Client::with_loopback_http_for_tests(Box::new(Refusing(refusal.clone())));
    let err = client
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert_eq!(err, refusal);
    assert_eq!(err.code(), Some(code));
}

#[rstest::rstest]
#[case::untrusted_helper(untrusted(), "credential-helper-untrusted")]
#[case::unknown_project(project_unknown(), "credential-helper-project-unknown")]
fn a_refused_helper_is_reported_when_a_basic_challenge_needs_a_login(
    #[case] refusal: OciError,
    #[case] code: &str,
) {
    let server =
        Server::start(|_| Response::new(401).header("WWW-Authenticate", "Basic realm=\"reg\""));
    let client = Client::with_loopback_http_for_tests(Box::new(Refusing(refusal.clone())));
    let err = client
        .fetch_manifest(&reference(&server, "a/b:1"))
        .unwrap_err();
    assert_eq!(err, refusal);
    assert_eq!(err.code(), Some(code));
}

mod config_key_case {
    use super::*;

    fn creds_with(cfg: &serde_json::Value) -> (tempfile::TempDir, DefaultCredentials) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), cfg.to_string()).unwrap();
        let creds = DefaultCredentials {
            docker_config_dir: Some(dir.path().to_path_buf()),
            ..DefaultCredentials::default()
        };
        (dir, creds)
    }

    #[cfg(unix)]
    #[test]
    fn a_cred_helpers_key_matches_the_host_case_insensitively_and_beats_creds_store() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut creds) = creds_with(&json!({
            "credHelpers": {"Registry.Example:5000": "mine"},
            "credsStore": "fallback",
        }));
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for (name, user) in [
            ("mine", "from-cred-helpers"),
            ("fallback", "from-creds-store"),
        ] {
            let p = bin.join(format!("docker-credential-{name}"));
            std::fs::write(
                &p,
                format!("#!/bin/sh\ncat >/dev/null; echo '{{\"Username\":\"{user}\",\"Secret\":\"s\"}}'\n"),
            )
            .unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        creds.helper_search_path = Some(bin.into_os_string());
        creds.project_root = Some(dir.path().join("project"));
        creds.helper_timeout = Some(std::time::Duration::from_secs(30));
        let user = |host: &str| creds.credential(host).unwrap().unwrap().username;
        assert_eq!(user("registry.example:5000"), "from-cred-helpers");
        assert_eq!(user("REGISTRY.EXAMPLE:5000"), "from-cred-helpers");
        assert_eq!(user("registry.example:5001"), "from-creds-store");
    }

    #[test]
    fn auths_keys_that_differ_only_in_host_case_and_disagree_are_refused() {
        let (_dir, creds) = creds_with(&json!({"auths": {
            "registry.example": {"auth": base64_std(b"alice:one")},
            "REGISTRY.example": {"auth": base64_std(b"bob:two")},
        }}));
        for host in ["registry.example", "Registry.Example"] {
            let err = creds.credential(host).unwrap_err();
            assert!(matches!(err, OciError::Credential(_)), "{err:?}");
            assert!(err.to_string().contains("conflicting"), "{err}");
        }
    }

    #[test]
    fn auths_keys_that_differ_only_in_host_case_and_agree_give_that_credential() {
        let (_dir, creds) = creds_with(&json!({"auths": {
            "registry.example": {"auth": base64_std(b"alice:one")},
            "REGISTRY.example": {"auth": base64_std(b"alice:one")},
        }}));
        let c = creds.credential("registry.example").unwrap().unwrap();
        assert_eq!((c.username.as_str(), c.secret.as_str()), ("alice", "one"));
    }

    #[test]
    fn cred_helpers_keys_that_differ_only_in_host_case_and_disagree_are_refused() {
        let (_dir, creds) = creds_with(&json!({"credHelpers": {
            "registry.example": "one",
            "REGISTRY.example": "two",
        }}));
        let err = creds.credential("registry.example").unwrap_err();
        assert!(matches!(err, OciError::Credential(_)), "{err:?}");
        assert!(err.to_string().contains("conflicting"), "{err}");
    }
}
