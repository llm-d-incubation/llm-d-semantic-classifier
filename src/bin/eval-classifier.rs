//! Offline evaluation harness for an anchor-based classifier definition.
//!
//! Mirrors the reference `classify_by_anchors` methodology used to fine-tune the
//! model (top-k mean cosine per label, softmax over label scores for
//! confidence) so llm-d-sc numbers are comparable to the training-side report.
//!
//! Usage:
//!   eval-classifier --model <dir> --classifier <json> --dataset <jsonl> [--json <out>]

use std::collections::BTreeMap;
use std::time::Instant;

use llm_d_sc::embedding::Embedder;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn arg(name: &str, default: Option<&str>) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| default.map(str::to_string))
        .unwrap_or_else(|| panic!("missing required argument {name}"))
}

fn main() {
    let model_dir = arg("--model", Some("artifacts/models/sensitivity"));
    let def_path = arg("--classifier", Some("classifiers/sensitivity.json"));
    let data_path = arg(
        "--dataset",
        Some("evals/datasets/sensitivity-heldout.jsonl"),
    );
    let out_path = arg("--json", Some(""));

    let def: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&def_path).expect("read classifier"))
            .unwrap();
    let labels: Vec<String> = def["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let top_k = def["top_k"].as_u64().unwrap_or(3) as usize;

    let embedder = Embedder::load(
        &format!("{model_dir}/config.json"),
        &format!("{model_dir}/model.safetensors"),
        &format!("{model_dir}/tokenizer.json"),
        &format!("{model_dir}/1_Pooling/config.json"),
    )
    .expect("load embedder");

    // Embed every anchor once, grouped by label.
    let anchors: Vec<(String, Vec<Vec<f32>>)> = labels
        .iter()
        .map(|l| {
            let vecs = def["anchors"][l]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| embedder.embed(t.as_str().unwrap()).expect("embed anchor"))
                .collect();
            (l.clone(), vecs)
        })
        .collect();

    // Load the held-out dataset.
    let raw = std::fs::read_to_string(&data_path).expect("read dataset");
    let items: Vec<serde_json::Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    let mut confusion: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut latencies_us: Vec<u128> = Vec::new();
    let mut correct = 0usize;
    let mut hard_total = 0usize;
    let mut hard_correct = 0usize;
    let mut errors: Vec<(String, String, String, f32)> = Vec::new();
    let mut confidences: Vec<(f32, bool)> = Vec::new();

    for item in &items {
        let text = item["text"].as_str().unwrap();
        let truth = item["tier"].as_str().unwrap().to_string();
        let hard = item["hard"].as_bool().unwrap_or(false);

        let t0 = Instant::now();
        let emb = embedder.embed(text).expect("embed input");
        // Score each label by the mean of its top-k anchor cosines.
        let mut scores: Vec<(String, f32)> = anchors
            .iter()
            .map(|(l, vecs)| {
                let mut sims: Vec<f32> = vecs.iter().map(|a| cosine(&emb, a)).collect();
                sims.sort_by(|x, y| y.partial_cmp(x).unwrap());
                let k = top_k.min(sims.len());
                (l.clone(), sims[..k].iter().sum::<f32>() / k as f32)
            })
            .collect();
        latencies_us.push(t0.elapsed().as_micros());

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let pred = scores[0].0.clone();

        // Softmax confidence over label scores (matches the reference evaluator).
        let max = scores.iter().map(|s| s.1).fold(f32::MIN, f32::max);
        let exps: Vec<f32> = scores.iter().map(|s| (s.1 - max).exp()).collect();
        let sum: f32 = exps.iter().sum();
        let confidence = exps[0] / sum;

        let ok = pred == truth;
        if ok {
            correct += 1;
        } else {
            errors.push((text.to_string(), truth.clone(), pred.clone(), confidence));
        }
        if hard {
            hard_total += 1;
            if ok {
                hard_correct += 1;
            }
        }
        confidences.push((confidence, ok));
        *confusion.entry((truth, pred)).or_insert(0) += 1;
    }

    let n = items.len();
    let accuracy = correct as f64 / n as f64;

    // Per-label precision / recall / F1.
    let mut f1s = Vec::new();
    println!("\nlabel            precision  recall      f1   support");
    println!("---------------------------------------------------");
    for l in &labels {
        let tp = *confusion.get(&(l.clone(), l.clone())).unwrap_or(&0) as f64;
        let fp: f64 = labels
            .iter()
            .map(|t| *confusion.get(&(t.clone(), l.clone())).unwrap_or(&0) as f64)
            .sum::<f64>()
            - tp;
        let fneg: f64 = labels
            .iter()
            .map(|p| *confusion.get(&(l.clone(), p.clone())).unwrap_or(&0) as f64)
            .sum::<f64>()
            - tp;
        let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        let r = if tp + fneg > 0.0 {
            tp / (tp + fneg)
        } else {
            0.0
        };
        let f1 = if p + r > 0.0 {
            2.0 * p * r / (p + r)
        } else {
            0.0
        };
        f1s.push(f1);
        println!(
            "{:<15} {:>9.3} {:>7.3} {:>7.3} {:>9}",
            l,
            p,
            r,
            f1,
            (tp + fneg) as usize
        );
    }
    let macro_f1 = f1s.iter().sum::<f64>() / f1s.len() as f64;

    println!("\nconfusion (rows = truth, cols = predicted)");
    print!("{:<15}", "");
    for l in &labels {
        print!("{:>13}", &l[..l.len().min(12)]);
    }
    println!();
    for t in &labels {
        print!("{:<15}", t);
        for p in &labels {
            print!(
                "{:>13}",
                confusion.get(&(t.clone(), p.clone())).unwrap_or(&0)
            );
        }
        println!();
    }

    latencies_us.sort();
    let p50 = latencies_us[latencies_us.len() / 2] as f64 / 1000.0;
    let p99 = latencies_us[(latencies_us.len() as f64 * 0.99) as usize % latencies_us.len()] as f64
        / 1000.0;

    println!("\nmodel            {model_dir}");
    println!("dataset          {data_path} (n={n})");
    println!("accuracy         {:.4}  ({correct}/{n})", accuracy);
    println!("macro f1         {:.4}", macro_f1);
    println!("misrouting rate  {:.4}", 1.0 - accuracy);
    if hard_total > 0 {
        println!(
            "hard-case acc    {:.4}  ({hard_correct}/{hard_total})",
            hard_correct as f64 / hard_total as f64
        );
    }
    println!("latency p50/p99  {p50:.2} ms / {p99:.2} ms  (embed + rank, single thread)");

    if !errors.is_empty() {
        println!("\nmisclassifications ({}):", errors.len());
        for (t, truth, pred, c) in errors.iter().take(25) {
            println!(
                "  {truth} -> {pred} (conf {c:.2})  \"{}\"",
                &t[..t.len().min(72)]
            );
        }
    }

    if !out_path.is_empty() {
        let conf_rows: Vec<Vec<usize>> = labels
            .iter()
            .map(|t| {
                labels
                    .iter()
                    .map(|p| *confusion.get(&(t.clone(), p.clone())).unwrap_or(&0))
                    .collect()
            })
            .collect();
        let report = serde_json::json!({
            "model": model_dir,
            "classifier": def_path,
            "dataset": data_path,
            "n": n,
            "accuracy": accuracy,
            "macro_f1": macro_f1,
            "hard_case_accuracy": if hard_total > 0 { hard_correct as f64 / hard_total as f64 } else { 0.0 },
            "latency_p50_ms": p50,
            "latency_p99_ms": p99,
            "labels": labels,
            "confusion": conf_rows,
        });
        std::fs::write(&out_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
        println!("\nreport written to {out_path}");
    }
}
