use super::{CandidateCatalog, SelectedCandidate, VerifiedCandidate, VerifiedCandidateSet};

pub(crate) fn verify_candidates(
    args: &crate::Args,
    catalog: &CandidateCatalog,
    candidates: &[SelectedCandidate],
) -> VerifiedCandidateSet {
    let mut results = Vec::new();
    for candidate in candidates {
        let prefix = catalog.prefix(candidate);
        let cycle = catalog.cycle(candidate);
        let layout = catalog.layout(candidate);
        let config = crate::SimConfig {
            ticks: args.ticks,
            start_x: candidate.start_offset,
            start_y: args.start_y,
            start_vx: args.start_vx,
            start_vy: args.start_vy,
            entity_id_mod4: candidate.entity_id_mod4,
            initial_tick_count: candidate.initial_tick_count,
            start_on_ground: Some(args.start_on_ground),
        };
        let sim = crate::simulate(&layout, &config);
        let Some(long) = crate::best_long_window(&sim, args.long_window) else {
            continue;
        };
        let suffix = crate::suffix_long_metrics(
            &sim,
            candidate.cadence.cadence_start_tick,
            args.long_window.min(args.ticks.saturating_sub(10)),
        );
        let Some(full) = crate::full_cadence_metrics(
            &sim,
            candidate.cadence.cadence_start_tick,
            args.full_cadence_pairs,
            args.full_cadence_tolerance,
        ) else {
            continue;
        };
        let pass = classify_verified_candidate(&candidate.cadence, &full, &long, suffix.as_ref());
        if pass == "weak" && !candidate.cadence.cadence_pass && !args.keep_weak {
            continue;
        }
        let score = verified_candidate_score(
            &candidate.cadence,
            &full,
            &long,
            suffix.as_ref(),
            prefix.cells.len(),
            &prefix.label,
            cycle.proven,
        );
        results.push(VerifiedCandidate {
            selected: candidate.clone(),
            pass,
            score,
            full_cadence: full,
            long_window: long,
            suffix_window: suffix,
            simulation: sim,
        });
    }
    VerifiedCandidateSet {
        long_verified: candidates.len(),
        candidates: results,
    }
}

fn candidate_score(
    early: &crate::EarlyCadence,
    long: &crate::WindowMetrics,
    suffix: Option<&crate::WindowMetrics>,
    prefix_length: usize,
    prefix_label: &str,
    proven: bool,
) -> f64 {
    let early_penalty = if early.cadence_pass { 0.0 } else { 10_000.0 };
    let long_penalty = long.mean_vx_error * 5000.0
        + long.std_vx * 100.0
        + (long.average_distance_vx - 0.5).abs() * 5000.0;
    let suffix_penalty = suffix
        .map(|value| {
            value.mean_vx_error * 3000.0
                + value.std_vx * 50.0
                + (value.average_distance_vx - 0.5).abs() * 3000.0
        })
        .unwrap_or(1000.0);
    let prefix_penalty = prefix_length as f64 * 2.0;
    let source_penalty = if prefix_label.contains('S') { 3.0 } else { 0.0 };
    let unproven_penalty = if proven { 0.0 } else { 20.0 };
    early_penalty
        + early.early_cadence_score
        + long_penalty
        + suffix_penalty
        + prefix_penalty
        + source_penalty
        + unproven_penalty
}

fn verified_candidate_score(
    early: &crate::EarlyCadence,
    full: &crate::FullCadence,
    long: &crate::WindowMetrics,
    suffix: Option<&crate::WindowMetrics>,
    prefix_length: usize,
    prefix_label: &str,
    proven: bool,
) -> f64 {
    let miss_penalty = (1.0 - full.full_cadence_block_hit_rate) * 100_000.0;
    let full_distance_penalty = (full.full_cadence_average_speed - 0.5).abs() * 20_000.0
        + full.full_cadence_mean_abs_distance_error * 1000.0
        + full.full_cadence_max_abs_distance_error * 100.0;
    candidate_score(early, long, suffix, prefix_length, prefix_label, proven)
        + miss_penalty
        + full_distance_penalty
}

fn classify_candidate(
    early: &crate::EarlyCadence,
    long: &crate::WindowMetrics,
    suffix: Option<&crate::WindowMetrics>,
) -> &'static str {
    let early_strong = early.cadence_pass
        && early.cadence_start_tick <= 5
        && early.cadence_mean_abs_distance_error <= 0.025
        && early.cadence_max_abs_distance_error <= 0.075;
    let long_strong = long.mean_vx_error <= 0.005
        && (long.average_distance_vx - 0.5).abs() <= 0.005
        && long.std_vx <= 0.04;
    let suffix_ok = suffix
        .map(|value| value.mean_vx_error <= 0.02 && (value.average_distance_vx - 0.5).abs() <= 0.02)
        .unwrap_or(false);
    if early_strong && long_strong && suffix_ok {
        return "strong";
    }
    if early.cadence_start_tick <= 5
        && early.cadence_block_hit_rate >= 0.95
        && long.mean_vx_error <= 0.01
    {
        return "usable";
    }
    "weak"
}

fn classify_verified_candidate(
    early: &crate::EarlyCadence,
    full: &crate::FullCadence,
    long: &crate::WindowMetrics,
    suffix: Option<&crate::WindowMetrics>,
) -> &'static str {
    let base = classify_candidate(early, long, suffix);
    if base == "strong"
        && full.full_cadence_block_hit_rate == 1.0
        && full.full_cadence_within_tolerance_rate >= 0.98
        && (full.full_cadence_average_speed - 0.5).abs() <= 0.001
    {
        return "strong";
    }
    if early.cadence_start_tick <= 5
        && full.full_cadence_block_hit_rate >= 0.999
        && full.full_cadence_within_tolerance_rate >= 0.95
        && (full.full_cadence_average_speed - 0.5).abs() <= 0.0025
    {
        return "usable";
    }
    "weak"
}
