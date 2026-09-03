# Round 3 — methodology fixes

Round 3 exists because an external review identified four defects that make the
rounds 1–2 cache-miss conclusions unsafe. Each is addressed by a concrete change
to the instrument, not by a caveat in the prose.

| Defect | Fix |
|---|---|
| `RAYON_NUM_THREADS` never controlled; vertical sweep moved 3 variables at once | `rayon_matrix.py` — Rayon × workers with CPU pinned, replicated |
| Closed-loop only; cannot observe queue explosion | `scbench --rate/--arrival` — true open-loop, constant or Poisson |
| One run per cell; 2 M requests ≠ 10 experiments | 3 repetitions/cell (5 in the Rayon matrix), randomised order, bootstrap 95% CI |
| Synthetic filler prompts, no semantic variance | `corpus.jsonl` — 200,000 frozen unique utterances, 12 domains |
| Premise checks written to an unarchived tmp file | `premises_passed` / `premise_notes` in every result JSON |

## The corpus

200,000 **unique** utterances (seed `20260903`, frozen and committed), spanning
networking, Kubernetes, security, small/large code, reasoning, general QA,
troubleshooting, conversation, malformed input, multilingual, and tool/JSON.
Length: min 2 B, p50 446 B, p95 8,744 B, max 18,717 B.

Every result JSON records `corpus_count` and `corpus_sha256`, so two arms can be confirmed to have seen the same population even though the corpus itself is regenerated rather than archived.

Uniqueness is enforced in the corpus so that *how often an utterance recurs* is
the driver's decision, not the corpus's — `--dist uniform|zipf|hotset|unique`.
Those are separate axes and conflating them would make "what the traffic says"
inseparable from "how often it repeats".

## Open-loop vs closed-loop

`--rate N` paces requests against the wall clock via a shared ticket dispenser, so
the offered rate is independent of response latency. Poisson spacing uses
inverse-transform sampling on an exponential, making the arrival process
memoryless rather than merely jittered.

Every result records `load_mode`, `offered_rate_rps` and `rate_attainment`. An arm
whose attainment falls below 0.98 **fails its premise check**: the generator could
not keep up, so the arm measured the driver rather than the target.

## Premise enforcement

`scbench` now writes `premises_passed` and `premise_notes` into the canonical
result JSON, checking rate attainment, a Little's-law cross-check
(`implied_mean` vs measured mean must agree within 0.7–1.4×, else the driver was
the limiter), and a minimum sample count for percentile validity.

## Measurement hygiene rules (learned the hard way)

Each of these exists because it was violated and produced a wrong number.

1. **A cache-miss arm must prove it missed.** Check llm-d-sc's own hit/miss
   counters, do not infer it from the label. An unsalted corpus walk measured a
   4.4% miss rate while claiming to be 100% novel — a cache-HIT measurement that
   reported plausible-looking throughput.
2. **One campaign owns the cluster.** Ad-hoc probes run against a pod that a
   campaign is driving contaminate both. Wait, or use a separate target.
3. **Compare only arms that differ in one parameter.** A "salted vs unsalted"
   comparison at concurrency 64 vs 32 measures concurrency, not salt.
4. **A knob is not applied until it is observed to change something.** Both
   `--context-bytes` and `--cache-mode` were silently ignored in corpus mode; the
   sweeps ran and produced smooth, publishable, meaningless curves.
5. **Percentiles need samples.** p99.9 from fewer than 1,000 requests is noise;
   the audit flags it rather than letting it into a table.

## Repetition counts by campaign

Stated explicitly because they differ, and an earlier revision of this file
claimed a single figure for all of them.

| Campaign | Reps/cell | Why |
|---|--:|---|
| Rayon matrix (`rayon_matrix.py`) | 5 | high run-to-run variance on the miss path |
| Round-3 campaigns (`campaign3.py`) | 3 | 85 arms; 3 reps kept the cluster window affordable |
| Corrected knee sweep (`knee_rerun.py`) | 3 | 54 cells across 3 stacks × 3 regimes × 6 rates |
| Coverage sweeps (`coverage.py`) | 1 | counter deltas, not distributions; repeated across rates instead |

## Corpus size

200,000 unique utterances, seed `20260903`. `gen_corpus.py` defaults to 200,000
so the default matches what was run — an earlier default of 20,000 disagreed with
every document, and since the corpus is regenerated rather than archived, the
default *is* the contract.
