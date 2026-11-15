# Pinned specifications

These files are test inputs. Each is pinned to a commit so a test result does not change when
an upstream branch moves.

- `sarif-schema-2.1.0.json` is the OASIS SARIF 2.1.0 JSON schema, copied unchanged from
  https://github.com/oasis-tcs/sarif-spec/blob/1742a253eb654bf8a4ed7c60eaef7412b2c23553/sarif-2.1/schema/sarif-schema-2.1.0.json
- `reviewdog-diagnostic.schema.json` is a hand transcription of `message Diagnostic` and the
  messages it uses in
  https://github.com/reviewdog/reviewdog/blob/87e520d8cd363fdd84386e440f8fcb919662b0d0/proto/rdf/reviewdog.proto
- `codeclimate-issue.schema.json` is a hand transcription of the issue, location and position
  sections of
  https://github.com/codeclimate/platform/blob/690633cb2a08839a5bfa350ed925ddb6de55bbdc/spec/analyzers/SPEC.md

The last two upstream specs publish no JSON Schema.
