//! SPIKE bench: run patterns over one input file under the engine chosen by
//! `SLOPVAC_SPIKE_ENGINE` (interp | jit | callout | rust), median of `--reps` runs.
//!
//! Usage: spike-regex-bench <patterns.jsonl> <input> <label> <out.jsonl> [--reps N]
//!        [--timeout S] [--pattern-input DIR]
//! patterns.jsonl: {"id","source","caseless","ucp","origin"} per line. With
//! `--pattern-input DIR`, the input for pattern `id` is `DIR/<id>.txt` when it exists
//! (per-pattern attack strings), else the main input.
//! Each finished pattern appends one JSON line and flushes, so a rerun resumes. A pattern
//! that runs longer than the timeout gets a `killed` record and the process exits 3; the
//! driver reruns it to go on with the next pattern.
#![allow(clippy::pedantic, missing_docs, reason = "spike")]

use std::collections::HashSet;
use std::io::Write as _;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

enum Re {
    Text(slopvac_text::Pattern),
    Raw(slopvac_pcre2::Regex),
}

impl Re {
    fn find_all(&self, t: &str) -> Result<Vec<std::ops::Range<usize>>, String> {
        match self {
            Re::Text(p) => p.find_all(t).map_err(|e| e.to_string()),
            Re::Raw(r) => r.find_all(t).map_err(|e| e.to_string()),
        }
    }
}

fn hwm_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(-1.0, |kib| kib / 1024.0)
}

fn digest(spans: &[std::ops::Range<usize>]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in spans {
        for v in [s.start as u64, s.end as u64] {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    format!("{}:{h:016x}", spans.len())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (pats, input, label, out) = (&args[1], &args[2], &args[3], &args[4]);
    let mut reps = 3usize;
    let mut timeout = 300u64;
    let mut per_dir: Option<String> = None;
    let mut fuzz = 0usize;
    let mut i = 5;
    while i < args.len() {
        match args[i].as_str() {
            "--reps" => reps = args[i + 1].parse().unwrap(),
            "--timeout" => timeout = args[i + 1].parse().unwrap(),
            "--pattern-input" => per_dir = Some(args[i + 1].clone()),
            "--fuzz" => fuzz = args[i + 1].parse().unwrap(),
            a => panic!("unknown arg {a}"),
        }
        i += 2;
    }
    let mode = std::env::var("SLOPVAC_SPIKE_ENGINE").unwrap_or_else(|_| "interp".into());
    let done: HashSet<String> = std::fs::read_to_string(out)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["label"] == label.as_str())
        .filter_map(|v| v["id"].as_str().map(str::to_owned))
        .collect();
    let main_text = std::fs::read_to_string(input).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .unwrap();
    for line in std::fs::read_to_string(pats).unwrap().lines() {
        let p: Value = serde_json::from_str(line).unwrap();
        let id = p["id"].as_str().unwrap().to_owned();
        if done.contains(&id) {
            continue;
        }
        let text = per_dir
            .as_ref()
            .map(|d| format!("{d}/{id}.txt"))
            .and_then(|path| std::fs::read_to_string(path).ok());
        if per_dir.is_some() && text.is_none() {
            continue;
        }
        let text = text.unwrap_or_else(|| main_text.clone());
        let _ = std::fs::write(
            format!("{out}.cur"),
            format!("{id}\t{label}\t{}", p["origin"].as_str().unwrap_or("")),
        );
        let len = text.len();
        let source = p["source"].as_str().unwrap().to_owned();
        let caseless = p["caseless"].as_bool().unwrap();
        let ucp = p["ucp"].as_bool().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let built = if ucp {
                slopvac_text::Pattern::new(&source, caseless)
                    .map(Re::Text)
                    .map_err(|e| e.to_string())
            } else {
                slopvac_pcre2::Regex::new(&source, slopvac_pcre2::Options::new().caseless(caseless))
                    .map(Re::Raw)
                    .map_err(|e| e.to_string())
            };
            let re = match built {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(json!({"compile_error": e}));
                    return;
                }
            };
            if fuzz > 0 {
                let mut errs = 0usize;
                for t in fuzz_inputs(&source, fuzz) {
                    errs += usize::from(re.find_all(&t).is_err());
                }
                let _ = tx.send(json!({"fuzz": fuzz, "match_errors": errs}));
                return;
            }
            let mut times = Vec::new();
            let mut result = String::new();
            for _ in 0..reps {
                let t0 = Instant::now();
                let r = re.find_all(&text);
                times.push(t0.elapsed().as_secs_f64());
                result = match r {
                    Ok(s) => digest(&s),
                    Err(e) => format!("ERR {e}"),
                };
                if times.iter().sum::<f64>() > 120.0 {
                    break;
                }
            }
            times.sort_by(f64::total_cmp);
            let _ = tx.send(json!({
                "median_s": times[times.len() / 2],
                "runs": times.len(),
                "result": result,
            }));
        });
        let rec = match rx.recv_timeout(Duration::from_secs(timeout)) {
            Ok(mut v) => {
                v["killed"] = Value::Null;
                v
            }
            Err(_) => json!({"killed": format!("> {timeout} s, killed")}),
        };
        let killed = !rec["killed"].is_null();
        let mut rec = rec;
        rec["id"] = json!(id);
        rec["origin"] = p["origin"].clone();
        rec["label"] = json!(label);
        rec["mode"] = json!(mode);
        rec["input_bytes"] = json!(len);
        rec["hwm_mib"] = json!(hwm_mib());
        writeln!(f, "{rec}").unwrap();
        f.flush().unwrap();
        if killed {
            std::process::exit(3);
        }
    }
}

/// Strings built from the pattern's own words and a fixed set of tricky pieces
/// (newlines, CRLF, case-fold specials, marks, quotes), some upper- or title-cased, some
/// with a trailing newline. Deterministic per pattern.
fn fuzz_inputs(source: &str, n: usize) -> Vec<String> {
    const PIECES: &[&str] = &[
        " ", " ", " ", "\n", "\r\n", "\n\n", "\t", "-", "--", "'", "’", ".", ",", ";", ":", "?",
        "!", "_", "1", "42", "٣", "²", "\u{A0}", "\u{2028}", "\u{85}", "(", ")", "[", "]", "\"",
        "“", "”", "‘", "#", "## ", "* ", "- ", "`", "```", "$ ", "=======", "<<<<<<< ", "/",
        "K", "\u{212A}", "ß", "SS", "\u{17F}", "İ", "ı", "é", "e\u{301}", "\u{200D}", "x",
        "word", "Word", "WORD", "a", "the", "The", "I", "it's", "e.g.", "i.e.", "U.S.", "👍🏽",
        "—", "–", "http://x.io/a", "<b>", "&amp;", "…",
    ];
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_bs = false;
    for c in source.chars() {
        if c.is_alphabetic() && !prev_bs {
            cur.push(c);
        } else {
            if cur.chars().count() >= 2 {
                words.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
        prev_bs = c == '\\' && !prev_bs;
    }
    if cur.chars().count() >= 2 {
        words.push(cur);
    }
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
    for b in source.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let mut next = move || {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        h
    };
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let k = 1 + (next() % 14) as usize;
        let mut s = String::new();
        for _ in 0..k {
            let r = next();
            if !words.is_empty() && r % 2 == 0 {
                let w = &words[(r >> 8) as usize % words.len()];
                match (r >> 4) % 6 {
                    0 => s.push_str(&w.to_uppercase()),
                    1 => {
                        let mut cs = w.chars();
                        if let Some(f) = cs.next() {
                            s.extend(f.to_uppercase());
                            s.push_str(cs.as_str());
                        }
                    }
                    2 => {
                        // drop the last letter: a near miss
                        let mut t = w.clone();
                        t.pop();
                        s.push_str(&t);
                    }
                    _ => s.push_str(w),
                }
            } else {
                s.push_str(PIECES[(r >> 8) as usize % PIECES.len()]);
            }
        }
        if next() % 4 == 0 {
            s.push('\n');
        }
        out.push(s);
    }
    out
}
