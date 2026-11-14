# Divergences from the Python reference

The Rust text crate matches `slopvac.analyze` except where listed here. The golden suite
(slopvac-8l4.2.7) should expect these differences.

| Area | Python | Rust | Why |
| --- | --- | --- | --- |
| Offset of a vertical list's lead-in sentence when the text starts with whitespace | 0 | the true byte offset | Python's offset does not slice back to the sentence. Tested in `list_lead_in_keeps_its_true_offset_after_leading_whitespace`. |
| Sentence offsets | code points | UTF-8 bytes | The position contract uses bytes. |
