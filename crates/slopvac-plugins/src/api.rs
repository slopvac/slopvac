//! Typed hook calls: batching, request building and result validation.

use std::ops::Range;

use serde::Serialize;
use serde_json::Value;

use crate::error::{CompleteError, PluginError, RpcError};
use crate::hooks::{
    CompleteRequest, Completion, Document, FindingsDocument, FixEdit, RecipeFile, Rendered, Segment,
    Tag, check_completions, match_entries, parse_edits, parse_error_object, parse_recipe,
    parse_rendered, parse_score, parse_segments, parse_tags,
};
use crate::host::Plugin;
use crate::messages::host::{HostPhrase, HostText};
use crate::protocol::{
    CompleteParams, ExtractParams, FixParams, HookKind, RecipeParams, RenderParams, ScoreParams,
    TagParams, request_len,
};

/// The outcome for one document of a batch.
#[derive(Debug, Clone, PartialEq)]
pub struct DocOutcome<T> {
    /// Document id as sent.
    pub id: String,
    /// The validated result, or why this document failed.
    pub outcome: Result<T, PluginError>,
}

/// Anything that carries a [`Document`].
pub trait HasDocument: Serialize + Clone {
    /// The document.
    fn document(&self) -> &Document;
    /// Drop the path, for plugins that were not granted `document-path`.
    fn strip_path(&mut self);
}

impl HasDocument for Document {
    fn document(&self) -> &Document {
        self
    }
    fn strip_path(&mut self) {
        self.path = None;
    }
}

impl HasDocument for FindingsDocument {
    fn document(&self) -> &Document {
        &self.document
    }
    fn strip_path(&mut self) {
        self.document.path = None;
    }
}

/// Split items into batches under both limits.
///
/// `sizes` holds each item's encoded size and `overhead` the size of a request
/// with no items. Returns the batches and the indexes of items that cannot fit
/// a message even alone.
#[must_use]
pub fn plan_batches(
    sizes: &[usize],
    overhead: usize,
    max_items: usize,
    max_bytes: usize,
) -> (Vec<Range<usize>>, Vec<usize>) {
    let mut batches = Vec::new();
    let mut oversize = Vec::new();
    let mut start = 0;
    let mut bytes = overhead;
    let mut count = 0;
    for (i, &size) in sizes.iter().enumerate() {
        if overhead.saturating_add(size) > max_bytes {
            if count > 0 {
                batches.push(start..i);
            }
            oversize.push(i);
            start = i + 1;
            bytes = overhead;
            count = 0;
            continue;
        }
        let with_comma = size + usize::from(count > 0);
        if count == max_items || bytes + with_comma > max_bytes {
            batches.push(start..i);
            start = i;
            bytes = overhead;
            count = 0;
        }
        bytes += size + usize::from(count > 0);
        count += 1;
    }
    if count > 0 {
        batches.push(start..sizes.len());
    }
    (batches, oversize)
}

impl Plugin {
    /// Call a documents hook over `docs`, split into batches.
    ///
    /// `overhead` is the size of the request with no documents, from [`request_len`]. `send`
    /// builds the request for one batch and sends it.
    fn documents_call<D, T>(
        &self,
        overhead: usize,
        docs: &[D],
        send: &dyn Fn(&[D]) -> Result<Value, PluginError>,
        parse: &dyn Fn(&Value, &D) -> Result<T, PluginError>,
    ) -> Result<Vec<DocOutcome<T>>, PluginError>
    where
        D: HasDocument,
    {
        let n = self.negotiated();
        let mut docs = docs.to_vec();
        if !n.document_path {
            docs.iter_mut().for_each(HasDocument::strip_path);
        }
        let encoded: Vec<usize> = docs
            .iter()
            .map(|d| serde_json::to_vec(d).map(|v| v.len()))
            .collect::<Result<_, _>>()
            .map_err(|e| {
                PluginError::Usage(
                    HostText::EncodeDocument {
                        error: e.to_string(),
                    }
                    .to_string(),
                )
            })?;
        let (batches, oversize) = plan_batches(
            &encoded,
            overhead,
            n.max_batch_documents,
            n.max_message_bytes,
        );
        let mut out: Vec<Option<DocOutcome<T>>> = (0..docs.len()).map(|_| None).collect();
        for &i in &oversize {
            out[i] = Some(DocOutcome {
                id: docs[i].document().id.clone(),
                outcome: Err(PluginError::MessageTooLarge(
                    HostText::DocumentTooLarge {
                        id: docs[i].document().id.clone(),
                    }
                    .to_string(),
                )),
            });
        }
        for range in batches {
            let batch = &docs[range.clone()];
            let result = send(batch)?;
            let ids: Vec<&str> = batch.iter().map(|d| d.document().id.as_str()).collect();
            let entries = match_entries(&result, "document", &ids)?;
            for ((slot, entry), doc) in out[range].iter_mut().zip(entries).zip(batch) {
                let outcome = match entry.get("error") {
                    Some(e) => Err(parse_error_object(e).map_or_else(|e| e, PluginError::Rpc)),
                    None => parse(entry, doc),
                };
                *slot = Some(DocOutcome {
                    id: doc.document().id.clone(),
                    outcome,
                });
            }
        }
        Ok(out.into_iter().flatten().collect())
    }

    /// Tag documents.
    ///
    /// # Errors
    /// A request-level failure. Per-document failures sit in the outcomes.
    pub fn tag(
        &self,
        hook: &str,
        docs: &[Document],
    ) -> Result<Vec<DocOutcome<Vec<Tag>>>, PluginError> {
        let kind = HookKind::Tagger;
        let empty: &[Document] = &[];
        let overhead = request_len(
            kind.method(),
            &TagParams {
                documents: empty,
                hook,
            },
        )?;
        let send = |batch: &[Document]| {
            self.call_with(
                kind,
                hook,
                &TagParams {
                    documents: batch,
                    hook,
                },
            )
        };
        self.documents_call(overhead, docs, &send, &|e, d| parse_tags(e, &d.text))
    }

    /// Split documents into prose segments.
    ///
    /// # Errors
    /// A request-level failure.
    pub fn extract(
        &self,
        hook: &str,
        docs: &[Document],
    ) -> Result<Vec<DocOutcome<Vec<Segment>>>, PluginError> {
        let kind = HookKind::Extractor;
        let empty: &[Document] = &[];
        let overhead = request_len(
            kind.method(),
            &ExtractParams {
                documents: empty,
                hook,
            },
        )?;
        let send = |batch: &[Document]| {
            self.call_with(
                kind,
                hook,
                &ExtractParams {
                    documents: batch,
                    hook,
                },
            )
        };
        self.documents_call(overhead, docs, &send, &|e, d| parse_segments(e, &d.text))
    }

    /// Ask a fixer for edits. Edits are checked against the sent findings.
    ///
    /// # Errors
    /// A request-level failure.
    pub fn fix(
        &self,
        hook: &str,
        docs: &[FindingsDocument],
    ) -> Result<Vec<DocOutcome<Vec<FixEdit>>>, PluginError> {
        let kind = HookKind::Fixer;
        let empty: &[FindingsDocument] = &[];
        let overhead = request_len(
            kind.method(),
            &FixParams {
                documents: empty,
                hook,
            },
        )?;
        let send = |batch: &[FindingsDocument]| {
            self.call_with(
                kind,
                hook,
                &FixParams {
                    documents: batch,
                    hook,
                },
            )
        };
        self.documents_call(overhead, docs, &send, &|e, d| {
            parse_edits(e, &d.document.text, &d.findings)
        })
    }

    /// Score documents. A document the scorer skips has `None`.
    ///
    /// # Errors
    /// A request-level failure.
    pub fn score(
        &self,
        hook: &str,
        docs: &[FindingsDocument],
    ) -> Result<Vec<DocOutcome<Option<f64>>>, PluginError> {
        let kind = HookKind::Scorer;
        let empty: &[FindingsDocument] = &[];
        let overhead = request_len(
            kind.method(),
            &ScoreParams {
                documents: empty,
                hook,
            },
        )?;
        let send = |batch: &[FindingsDocument]| {
            self.call_with(
                kind,
                hook,
                &ScoreParams {
                    documents: batch,
                    hook,
                },
            )
        };
        self.documents_call(overhead, docs, &send, &|e, _| parse_score(e))
    }

    /// Render one report.
    ///
    /// # Errors
    /// Any failure. The returned text is untrusted: sanitise it for a terminal.
    pub fn render(
        &self,
        hook: &str,
        report: &Value,
        options: &Value,
    ) -> Result<Rendered, PluginError> {
        let params = RenderParams {
            options,
            report,
            reporter: hook,
        };
        parse_rendered(&self.call_with(HookKind::Reporter, hook, &params)?)
    }

    /// Ask a provider. Output is untrusted model text and has no route to a
    /// fixer or recipe.
    ///
    /// # Errors
    /// A request-level failure.
    pub fn complete(
        &self,
        hook: &str,
        request: &CompleteRequest,
    ) -> Result<Vec<Result<Completion, RpcError>>, PluginError> {
        self.complete_checked(hook, request).map_err(PluginError::from)
    }

    /// [`Self::complete`], with the stage of a failure kept: a refused result is
    /// [`CompleteError::Refused`], and every other failure is [`CompleteError::Host`]. The
    /// answers follow the order of the request's items.
    ///
    /// # Errors
    /// A request-level failure, by stage. A request without a model is `plugin-usage`, because
    /// `provider/complete` needs one.
    pub fn complete_checked(
        &self,
        hook: &str,
        request: &CompleteRequest,
    ) -> Result<Vec<Result<Completion, RpcError>>, CompleteError> {
        let model = request.model.as_deref().ok_or_else(|| {
            CompleteError::Host(PluginError::Usage(
                HostPhrase::CompleteModelMissing.as_static().to_owned(),
            ))
        })?;
        let params = CompleteParams {
            items: &request.items,
            model,
            provider: hook,
            task: request.task,
        };
        let result = self
            .call_with(HookKind::Provider, hook, &params)
            .map_err(CompleteError::Host)?;
        let ids: Vec<&str> = request.items.iter().map(|i| i.id.as_str()).collect();
        check_completions(&result, &ids)
    }

    /// Ask a recipe for a plan. The plan writes nothing.
    ///
    /// # Errors
    /// Any failure, including a path that escapes the root.
    pub fn recipe(
        &self,
        hook: &str,
        harness: &str,
        options: &Value,
    ) -> Result<Vec<RecipeFile>, PluginError> {
        let params = RecipeParams {
            harness,
            options,
            recipe: hook,
        };
        parse_recipe(&self.call_with(HookKind::Recipe, hook, &params)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_items_split_by_count() {
        let (b, o) = plan_batches(&[10; 5], 100, 2, 10_000);
        assert_eq!(b, vec![0..2, 2..4, 4..5]);
        assert_eq!(o, Vec::<usize>::new());
    }

    #[test]
    fn items_split_by_bytes_including_commas() {
        // overhead 100 + 40 + ',' + 40 = 181 fits; a third item would make 222.
        let (b, _) = plan_batches(&[40, 40, 40], 100, 64, 181);
        assert_eq!(b, vec![0..2, 2..3]);
    }

    #[test]
    fn an_item_that_cannot_fit_alone_is_reported_and_skipped() {
        let (b, o) = plan_batches(&[10, 500, 10], 100, 64, 200);
        assert_eq!(b, vec![0..1, 2..3]);
        assert_eq!(o, vec![1]);
    }
}
