//! `llm-d-sc-classify`: classify text against a built-in or custom taxonomy.
//!
//! The demonstration entry point for the runtime. It performs the same work the
//! served gRPC path performs -- resolve a classifier definition, embed its
//! anchors once, then embed the input and rank -- without requiring a server.
//!
//! Usage:
//!   llm-d-sc-classify "how do I center a div"
//!   llm-d-sc-classify --classifier sensitivity "here is our production API key"
//!   llm-d-sc-classify --classifier ./my-taxonomy.json --model ./artifacts/models/x "text"
//!   echo "one prompt per line" | llm-d-sc-classify --classifier cost

use std::io::{BufRead, Write};
use std::time::Instant;

use llm_d_sc::classify::{ClassificationInput, ClassifierRuntime, CandleClassifier};
use llm_d_sc::taxonomy::{built_in_names, ClassifierDefinition, DEFAULT_CLASSIFIER};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "llm-d-sc-classify: rank text against a semantic taxonomy\n\n\
             usage: llm-d-sc-classify [--classifier <name|path>] [--model <dir>] [--json] [text...]\n\n\
             built-in classifiers: {}\n\
             default: {DEFAULT_CLASSIFIER}\n\n\
             With no text argument, reads one prompt per line from stdin.",
            built_in_names().join(", ")
        );
        return;
    }

    let flag = |name: &str| -> Option<String> {
        argv.iter().position(|a| a == name).and_then(|i| argv.get(i + 1).cloned())
    };
    let json_out = argv.iter().any(|a| a == "--json");
    let spec = flag("--classifier").unwrap_or_else(|| DEFAULT_CLASSIFIER.to_string());

    let definition = match ClassifierDefinition::resolve(&spec) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("llm-d-sc-classify: {e}");
            std::process::exit(2);
        }
    };

    // A classifier definition names the model it was calibrated against, so the
    // default model directory follows the classifier rather than the other way
    // around. Mismatching them silently would produce confident nonsense.
    let model_dir = flag("--model")
        .unwrap_or_else(|| format!("artifacts/models/{}", definition.classifier_id));

    // Positional text = every argument that is not a flag or a flag's value.
    let mut skip_next = false;
    let text_args: Vec<String> = argv
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if a.starts_with("--") {
                skip_next = *a != "--json";
                return false;
            }
            true
        })
        .map(|(_, a)| a.clone())
        .collect();

    let load_start = Instant::now();
    let classifier =
        match CandleClassifier::from_modelcar_with(std::path::Path::new(&model_dir), definition.clone())
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "llm-d-sc-classify: could not load '{}' from {model_dir}: {e}\n\
                     hint: ./hack/fetch-model --classifier {}",
                    definition.classifier_id, definition.classifier_id
                );
                std::process::exit(1);
            }
        };
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

    if !json_out {
        eprintln!(
            "classifier {} ({} labels, {} anchors, taxonomy {})  loaded in {load_ms:.0} ms",
            definition.classifier_id,
            definition.labels.len(),
            definition.anchor_count(),
            definition.taxonomy_revision,
        );
    }

    let inputs: Vec<String> = if !text_args.is_empty() {
        vec![text_args.join(" ")]
    } else {
        std::io::stdin().lock().lines().map_while(Result::ok)
            .filter(|l| !l.trim().is_empty()).collect()
    };
    if inputs.is_empty() {
        eprintln!("llm-d-sc-classify: no input text (pass text or pipe it on stdin)");
        std::process::exit(2);
    }

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for text in inputs {
        let t0 = Instant::now();
        let result = classifier.classify(ClassificationInput {
            text: text.clone(),
            requested_signals: vec![definition.signal.clone()],
            session_metadata: Default::default(),
        });
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        match result {
            Err(e) => eprintln!("  error: {e}"),
            Ok(r) => {
                if json_out {
                    let obj = serde_json::json!({
                        "text": text,
                        "classifier_id": r.classifier_id,
                        "model_revision": r.model_revision,
                        "taxonomy_revision": r.taxonomy_revision,
                        "latency_ms": ms,
                        "ranked": r.ranked.iter()
                            .map(|s| serde_json::json!({"label": s.id, "score": s.score}))
                            .collect::<Vec<_>>(),
                    });
                    writeln!(out, "{}", serde_json::to_string(&obj).unwrap()).ok();
                } else {
                    writeln!(out, "\n  \"{text}\"").ok();
                    let top = r.ranked.first().map(|s| s.score).unwrap_or(0.0);
                    let second = r.ranked.get(1).map(|s| s.score).unwrap_or(0.0);
                    for (i, s) in r.ranked.iter().enumerate() {
                        // Bar is relative to the top score, so the shape of the
                        // ranking (decisive vs ambiguous) is visible at a glance.
                        let frac = if top > 0.0 { (s.score / top).clamp(0.0, 1.0) } else { 0.0 };
                        let bar = "#".repeat((frac * 24.0).round() as usize);
                        let mark = if i == 0 { "->" } else { "  " };
                        writeln!(out, "  {mark} {:<14} {:>6.3}  {bar}", s.id, s.score).ok();
                    }
                    writeln!(out, "     margin {:.3}   {ms:.1} ms", top - second).ok();
                }
            }
        }
    }
}
