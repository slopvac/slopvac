//! The OCI client against the live CI package on GHCR.
//!
//! Every test pushes under a tag that holds the workflow run id and attempt, so runs never
//! share a tag. The `oci-live` workflow runs these with `--run-ignored only`, with
//! `GITHUB_TOKEN`, `GITHUB_RUN_ID` and `GITHUB_RUN_ATTEMPT` set. A missing variable fails the
//! test; nothing passes by skipping. Only the `oci-live` workflow pushes to the package.

use std::collections::BTreeMap;
use std::error::Error;

use slopvac_plugins::oci::{
    Anonymous, Client, DefaultCredentials, Descriptor, Digest, EnvCredentials, Index, MEDIA_EMPTY,
    MEDIA_INDEX, MEDIA_MANIFEST, Manifest, Reference, Target,
};

type TestResult = Result<(), Box<dyn Error>>;

const PACKAGE: &str = "ghcr.io/slopvac/oci-client-ci";
const ARTIFACT_TYPE: &str = "application/vnd.slopvac.ci.test.v1";
const REFERRER_TYPE: &str = "application/vnd.slopvac.ci.referrer.v1";
const LAYER_TYPE: &str = "application/vnd.slopvac.ci.layer.v1";

fn var(name: &str) -> Result<String, Box<dyn Error>> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(format!("{name} must be set for the live GHCR tests").into()),
    }
}

/// One run's view of the package: an authenticated client and a tag stem unique to the run.
struct Live {
    client: Client,
    stem: String,
}

impl Live {
    fn new() -> Result<Self, Box<dyn Error>> {
        // The client's own credential chain reads the token; only the lookup is ours.
        let creds = DefaultCredentials {
            env: EnvCredentials {
                github_token: Some(var("GITHUB_TOKEN")?),
                ..EnvCredentials::default()
            },
            ..DefaultCredentials::default()
        };
        Ok(Self {
            client: Client::with_credentials(Box::new(creds)),
            stem: format!(
                "ci-{}-{}",
                var("GITHUB_RUN_ID")?,
                var("GITHUB_RUN_ATTEMPT")?
            ),
        })
    }

    /// `ghcr.io/slopvac/oci-client-ci:ci-<run>-<attempt>-<name>`
    fn reference(&self, name: &str) -> Result<Reference, Box<dyn Error>> {
        Ok(Reference::parse(&format!(
            "{PACKAGE}:{}-{name}",
            self.stem
        ))?)
    }

    /// Push one layer and one manifest under the tag for `name`. The layer text names the
    /// tag, so no two tests push the same blob.
    fn push_artifact(
        &self,
        name: &str,
        subject: Option<Descriptor>,
    ) -> Result<Pushed, Box<dyn Error>> {
        let r = self.reference(name)?;
        let layer_bytes =
            format!("slopvac oci live test layer {}-{name}\n", self.stem).into_bytes();
        let config = self.client.push_blob(&r, MEDIA_EMPTY, b"{}")?;
        let layer = self.client.push_blob(&r, LAYER_TYPE, &layer_bytes)?;
        let manifest = Manifest {
            schema_version: 2,
            media_type: Some(MEDIA_MANIFEST.into()),
            artifact_type: Some(
                if subject.is_some() {
                    REFERRER_TYPE
                } else {
                    ARTIFACT_TYPE
                }
                .into(),
            ),
            config,
            layers: vec![layer.clone()],
            subject,
            annotations: BTreeMap::new(),
        };
        let body = serde_json::to_vec(&manifest)?;
        let digest = self.client.push_manifest(&r, MEDIA_MANIFEST, &body)?;
        Ok(Pushed {
            reference: r,
            layer,
            layer_bytes,
            body,
            digest,
        })
    }
}

struct Pushed {
    reference: Reference,
    layer: Descriptor,
    layer_bytes: Vec<u8>,
    body: Vec<u8>,
    digest: Digest,
}

impl Pushed {
    fn descriptor(&self, artifact_type: Option<&str>) -> Descriptor {
        Descriptor {
            artifact_type: artifact_type.map(str::to_owned),
            ..Descriptor::for_bytes(MEDIA_MANIFEST, &self.body)
        }
    }
}

#[test]
#[ignore = "needs a live GHCR package; run by the oci-live workflow"]
fn a_pushed_manifest_comes_back_by_tag_and_by_digest_with_the_local_digest() -> TestResult {
    let live = Live::new()?;
    let pushed = live.push_artifact("manifest", None)?;
    let local = Digest::of(&pushed.body);
    assert_eq!(
        pushed.digest, local,
        "push_manifest returns the local digest"
    );

    let by_tag = live.client.fetch_manifest(&pushed.reference)?;
    assert_eq!(by_tag.digest, local);
    assert_eq!(by_tag.body, pushed.body);

    let by_digest = live
        .client
        .fetch_manifest(&pushed.reference.with_target(Target::Digest(local.clone())))?;
    assert_eq!(by_digest.digest, local);
    assert_eq!(by_digest.body, pushed.body);
    Ok(())
}

#[test]
#[ignore = "needs a live GHCR package; run by the oci-live workflow"]
fn a_pushed_blob_is_pulled_back_byte_for_byte_and_checked_against_its_digest() -> TestResult {
    let live = Live::new()?;
    let pushed = live.push_artifact("blob", None)?;
    assert_eq!(pushed.layer.digest, Digest::of(&pushed.layer_bytes));

    let mut out = Vec::new();
    live.client
        .pull_blob(&pushed.reference, &pushed.layer, &mut out)?;
    assert_eq!(out, pushed.layer_bytes);
    Ok(())
}

// GHCR has no referrers API, so `referrers` takes the tag-schema fallback: it reads the
// image index under `sha256-<hex of the subject>`. The client exposes no signal for which
// path it took, so the test asserts the result, and that the index sits under that tag.
#[test]
#[ignore = "needs a live GHCR package; run by the oci-live workflow"]
fn referrers_find_a_manifest_through_the_tag_schema_fallback() -> TestResult {
    let live = Live::new()?;
    let subject = live.push_artifact("subject", None)?;
    let referrer = live.push_artifact("referrer", Some(subject.descriptor(None)))?;
    let listed = referrer.descriptor(Some(REFERRER_TYPE));

    let tag_schema = Index {
        schema_version: 2,
        media_type: Some(MEDIA_INDEX.into()),
        manifests: vec![listed.clone()],
    };
    let fallback_ref = subject
        .reference
        .with_target(Target::Tag(format!("sha256-{}", subject.digest.hex())));
    live.client.push_manifest(
        &fallback_ref,
        MEDIA_INDEX,
        &serde_json::to_vec(&tag_schema)?,
    )?;

    let found = live
        .client
        .referrers(&subject.reference, &subject.digest, None)?;
    assert_eq!(found, vec![listed.clone()]);

    let filtered =
        live.client
            .referrers(&subject.reference, &subject.digest, Some(REFERRER_TYPE))?;
    assert_eq!(filtered, vec![listed]);

    let other = live
        .client
        .referrers(&subject.reference, &subject.digest, Some(ARTIFACT_TYPE))?;
    assert!(
        other.is_empty(),
        "a filter on another artifact type matches nothing"
    );
    Ok(())
}

#[test]
#[ignore = "needs a live GHCR package; run by the oci-live workflow"]
fn the_tag_list_holds_a_tag_this_run_pushed() -> TestResult {
    let live = Live::new()?;
    let pushed = live.push_artifact("tags", None)?;
    let Target::Tag(tag) = &pushed.reference.target else {
        return Err("the pushed reference has a tag".into());
    };

    let tags = live.client.list_tags(&pushed.reference)?;
    assert!(
        tags.contains(tag),
        "{tag} is missing from {} tags",
        tags.len()
    );
    Ok(())
}

// The workflow skips this one, and only this one, on the first run, while the package is
// still private. Every other run fails when the anonymous pull fails.
#[test]
#[ignore = "needs a live GHCR package; run by the oci-live workflow"]
fn anonymous_pull_of_a_manifest_and_its_blob_works_on_the_public_package() -> TestResult {
    let live = Live::new()?;
    let pushed = live.push_artifact("anonymous", None)?;
    let anonymous = Client::with_credentials(Box::new(Anonymous));

    let fetched = anonymous.fetch_manifest(&pushed.reference)?;
    assert_eq!(fetched.digest, pushed.digest);

    let mut out = Vec::new();
    anonymous.pull_blob(&pushed.reference, &pushed.layer, &mut out)?;
    assert_eq!(out, pushed.layer_bytes);
    Ok(())
}
