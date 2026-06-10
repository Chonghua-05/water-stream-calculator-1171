use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use super::import::{
    prepare_verified_for_import, selected_to_search_payload, verification_queue,
    viewer_ready_to_search_payload,
};
use super::{
    CandidateCatalog, CandidateRef, PreparedSearch, SelectedCandidate, SelectedCandidateSet,
};

pub(crate) fn run(args: &crate::Args) -> crate::SearchPayload {
    let PreparedSearch {
        catalog,
        mut selected,
        ranked_early_candidates,
    } = prepare(args);

    if matches!(args.mode, crate::Mode::Early) {
        selected.candidates = ranked_early_candidates;
        return selected_to_search_payload(args, &catalog, selected);
    }

    let candidates_to_verify = verification_queue(args, &ranked_early_candidates);
    let mut verified_candidates = super::verify::verify_candidates(args, &catalog, &candidates_to_verify);
    verified_candidates
        .candidates
        .sort_by(|left, right| {
            left.score
                .total_cmp(&right.score)
                .then_with(|| left.selected.id.cmp(&right.selected.id))
        });
    let viewer_ready = prepare_verified_for_import(&catalog, verified_candidates);
    viewer_ready_to_search_payload(&selected, viewer_ready, ranked_early_candidates.len())
}

pub(crate) fn prepare(args: &crate::Args) -> PreparedSearch {
    prepare_cancelable(args, None, None).expect("prepare without cancellation should not fail")
}

pub(crate) fn prepare_cancelable(
    args: &crate::Args,
    cancel: Option<&AtomicBool>,
    mut progress: Option<&mut dyn FnMut(super::PrepareProgress)>,
) -> Result<PreparedSearch, String> {
    let catalog = CandidateCatalog::new(args);
    let start_offsets = start_offsets(args);
    let early_ticks = early_ticks(args);
    let mut selected =
        collect_selected_candidates(args, &catalog, &start_offsets, early_ticks, cancel, &mut progress)?;
    selected
        .candidates
        .sort_by(compare_selected_candidates);

    let ranked_early_candidates = if args.dedupe_long {
        dedupe_selected_candidates(std::mem::take(&mut selected.candidates), &catalog)
    } else {
        std::mem::take(&mut selected.candidates)
    };

    Ok(PreparedSearch {
        catalog,
        selected,
        ranked_early_candidates,
    })
}

#[derive(Clone)]
struct FrontierState {
    indices: Vec<usize>,
    length: usize,
    signature: String,
    frontier_score: f64,
    last_atom_index: Option<usize>,
    cadence_start_tick: usize,
    block_hit_rate: f64,
    within_tolerance_rate: f64,
    mean_signed_distance_error: f64,
    transient_shape_mae16: Option<f64>,
    transient_shape_mae24: Option<f64>,
}

#[derive(Clone)]
struct PrefixExpansion {
    indices: Vec<usize>,
    last_atom_index: Option<usize>,
}

struct PrefixEvaluation {
    frontier: Option<FrontierState>,
    evaluated: usize,
    kept: usize,
    candidates: Vec<SelectedCandidate>,
}

fn collect_selected_candidates(
    args: &crate::Args,
    catalog: &CandidateCatalog,
    start_offsets: &[f64],
    early_ticks: usize,
    cancel: Option<&AtomicBool>,
    progress: &mut Option<&mut dyn FnMut(super::PrepareProgress)>,
) -> Result<SelectedCandidateSet, String> {
    let mut evaluated = 0_usize;
    let mut kept = 0_usize;
    let mut early_candidates = Vec::new();
    let retained_limit = retained_early_candidate_limit(args);
    let atoms = crate::prefix_atoms();
    let empty = PrefixExpansion {
        indices: Vec::new(),
        last_atom_index: None,
    };
    let empty_eval = evaluate_prefix(
        args,
        catalog,
        start_offsets,
        early_ticks,
        &atoms,
        &empty,
        cancel,
    )?;
    evaluated += empty_eval.evaluated;
    kept += empty_eval.kept;
    merge_candidates(
        &mut early_candidates,
        empty_eval.candidates,
        retained_limit,
    );

    let mut frontier = vec![FrontierState {
        indices: Vec::new(),
        length: 0,
        signature: String::new(),
        frontier_score: 0.0,
        last_atom_index: None,
        cadence_start_tick: 0,
        block_hit_rate: 0.0,
        within_tolerance_rate: 0.0,
        mean_signed_distance_error: 0.0,
        transient_shape_mae16: None,
        transient_shape_mae24: None,
    }];
    let mut seen_signatures = HashSet::from([String::new()]);

    for layer in 1..=args.max_prefix {
        if is_cancelled(cancel) {
            return Err("search task cancelled".to_string());
        }
        if frontier.is_empty() {
            break;
        }
        let frontier_in = frontier.len();
        let expansions = build_frontier_expansions(&frontier, &atoms, args.max_prefix, &mut seen_signatures);
        if expansions.is_empty() {
            break;
        }
        let expanded_prefixes = expansions.len();
        if let Some(callback) = progress.as_deref_mut() {
            callback(super::PrepareProgress {
                layer,
                frontier_in,
                processed_prefixes: 0,
                expanded_prefixes,
                frontier_out: frontier.len(),
                evaluated_total: evaluated,
                kept_total: kept,
            });
        }
        let worker_count = args.workers.max(1).min(expanded_prefixes.max(1));
        let chunk_size = prefix_chunk_size(args.workers, expanded_prefixes);
        let mut next_frontier = Vec::with_capacity(expanded_prefixes.min(args.beam_width.saturating_mul(16).max(64)));
        let mut processed_total = 0_usize;

        for chunk in expansions.chunks(chunk_size) {
            if is_cancelled(cancel) {
                return Err("search task cancelled".to_string());
            }
            let chunk_results = evaluate_prefix_chunk(
                args,
                catalog,
                start_offsets,
                early_ticks,
                &atoms,
                chunk,
                cancel,
                worker_count,
            )?;
            for result in chunk_results {
                evaluated += result.evaluated;
                kept += result.kept;
                if let Some(state) = result.frontier {
                    next_frontier.push(state);
                }
                merge_candidates(&mut early_candidates, result.candidates, retained_limit);
            }
            processed_total += chunk.len();
            if let Some(callback) = progress.as_deref_mut() {
                callback(super::PrepareProgress {
                    layer,
                    frontier_in,
                    processed_prefixes: processed_total,
                    expanded_prefixes,
                    frontier_out: 0,
                    evaluated_total: evaluated,
                    kept_total: kept,
                });
            }
        }
        frontier = prune_frontier(next_frontier, args);
        if let Some(callback) = progress.as_deref_mut() {
            callback(super::PrepareProgress {
                layer,
                frontier_in,
                processed_prefixes: expanded_prefixes,
                expanded_prefixes,
                frontier_out: frontier.len(),
                evaluated_total: evaluated,
                kept_total: kept,
            });
        }
    }

    Ok(SelectedCandidateSet {
        evaluated,
        kept,
        candidates: early_candidates,
    })
}

fn early_candidate_score(early: &crate::EarlyCadence, prefix_length: usize) -> f64 {
    early.cadence_mean_abs_distance_error * 1000.0
        + early.cadence_max_abs_distance_error * 250.0
        + (1.0 - early.cadence_block_hit_rate) * 1000.0
        + (1.0 - early.cadence_within_tolerance_rate) * 500.0
        + ((early.cadence_start_tick as i32 - 2).max(0) as f64) * 10.0
        + prefix_length as f64 * 0.5
}

fn transient_shape_metrics(
    args: &crate::Args,
    start_offset: f64,
    sim: &crate::Simulation,
) -> (Option<f64>, Option<f64>) {
    if !crate::uses_post_piston_game2_reference(start_offset, args.start_vx, args.start_on_ground) {
        return (None, None);
    }
    (
        crate::post_piston_game2_transient_mae(&sim.xs, crate::TRANSIENT_SHAPE_WINDOW_SHORT),
        crate::post_piston_game2_transient_mae(&sim.xs, crate::TRANSIENT_SHAPE_WINDOW_LONG),
    )
}

fn transient_shape_penalty(mae16: Option<f64>, mae24: Option<f64>) -> f64 {
    mae16.unwrap_or(0.0) * crate::TRANSIENT_SHAPE_SCORE_WEIGHT
        + mae24.unwrap_or(0.0) * (crate::TRANSIENT_SHAPE_SCORE_WEIGHT * 0.375)
}

fn retained_early_candidate_limit(args: &crate::Args) -> Option<usize> {
    let base_limit = if args.early_limit > 0 {
        args.early_limit
    } else if args.long_limit > 0 {
        args.long_limit
    } else if args.top > 0 {
        args.top
    } else {
        0
    };
    if base_limit == 0 {
        return None;
    }
    let duplicate_factor = args.entity_id_mods.len().max(1) * args.initial_tick_counts.len().max(1);
    Some(
        base_limit
            .saturating_mul(duplicate_factor)
            .saturating_mul(64)
            .max(base_limit),
    )
}

fn merge_candidates(
    target: &mut Vec<SelectedCandidate>,
    mut incoming: Vec<SelectedCandidate>,
    limit: Option<usize>,
) {
    if incoming.is_empty() {
        return;
    }
    target.append(&mut incoming);
    if let Some(limit) = limit {
        if target.len() > limit {
            partition_early_candidates(target, limit);
        }
    }
}

fn partition_early_candidates(candidates: &mut Vec<SelectedCandidate>, limit: usize) {
    if candidates.len() <= limit {
        return;
    }
    candidates.select_nth_unstable_by(limit, compare_selected_candidates);
    candidates.truncate(limit);
}

fn dedupe_key(candidate: &SelectedCandidate, catalog: &CandidateCatalog) -> String {
    format!(
        "{}|{}||{:.9}||{}",
        candidate.prefix.signature,
        catalog.cycles[candidate.address.cycle_index].signature,
        candidate.start_offset,
        candidate.initial_tick_count
    )
}

fn dedupe_selected_candidates(
    candidates: Vec<SelectedCandidate>,
    catalog: &CandidateCatalog,
) -> Vec<SelectedCandidate> {
    let mut ordered: Vec<SelectedCandidate> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    for candidate in candidates {
        let key = dedupe_key(&candidate, catalog);
        if let Some(&position) = positions.get(&key) {
            if compare_selected_candidates(&candidate, &ordered[position]).is_lt() {
                ordered[position] = candidate;
            }
            continue;
        }
        positions.insert(key, ordered.len());
        ordered.push(candidate);
    }
    ordered.sort_by(compare_selected_candidates);
    ordered
}

fn start_offsets(args: &crate::Args) -> Vec<f64> {
    if let Some(offsets) = args.fixed_start_offsets.clone() {
        offsets
    } else {
        (0..args.start_samples)
            .map(|index| 0.125 + index as f64 * (0.75 / (args.start_samples - 1) as f64))
            .collect()
    }
}

fn early_ticks(args: &crate::Args) -> usize {
    args.ticks.min(5 + args.cadence_pairs * 2 + 4)
}

fn build_frontier_expansions(
    frontier: &[FrontierState],
    atoms: &[crate::PrefixAtom],
    max_cells: usize,
    seen_signatures: &mut HashSet<String>,
) -> Vec<PrefixExpansion> {
    let mut expansions = Vec::new();
    for state in frontier {
        for (atom_index, atom) in atoms.iter().enumerate() {
            let next_length = state.length + atom.cells.len();
            if next_length > max_cells {
                continue;
            }
            let mut next_indices = state.indices.clone();
            next_indices.push(atom_index);
            let prefix = crate::prefix_spec_from_indices(&next_indices, atoms);
            if !seen_signatures.insert(prefix.signature) {
                continue;
            }
            expansions.push(PrefixExpansion {
                indices: next_indices,
                last_atom_index: Some(atom_index),
            });
        }
    }
    expansions
}

fn prefix_chunk_size(workers: usize, total: usize) -> usize {
    if total <= 16 {
        return total.max(1);
    }
    workers
        .max(1)
        .saturating_mul(16)
        .clamp(64, 256)
        .min(total)
}

fn evaluate_prefix_chunk(
    args: &crate::Args,
    catalog: &CandidateCatalog,
    start_offsets: &[f64],
    early_ticks: usize,
    atoms: &[crate::PrefixAtom],
    chunk: &[PrefixExpansion],
    cancel: Option<&AtomicBool>,
    worker_count: usize,
) -> Result<Vec<PrefixEvaluation>, String> {
    if chunk.is_empty() {
        return Ok(Vec::new());
    }
    let next_index = AtomicUsize::new(0);
    let collected = Mutex::new(Vec::with_capacity(chunk.len()));
    let first_error = Mutex::new(None::<String>);

    thread::scope(|scope| {
        for _ in 0..worker_count.min(chunk.len().max(1)) {
            let collected = &collected;
            let first_error = &first_error;
            scope.spawn(|| {
                let mut local = Vec::new();
                loop {
                    if is_cancelled(cancel) {
                        break;
                    }
                    let index = next_index.fetch_add(1, Ordering::Relaxed);
                    if index >= chunk.len() {
                        break;
                    }
                    match evaluate_prefix(
                        args,
                        catalog,
                        start_offsets,
                        early_ticks,
                        atoms,
                        &chunk[index],
                        cancel,
                    ) {
                        Ok(value) => local.push(value),
                        Err(error) => {
                            let mut shared = first_error.lock().expect("frontier error mutex poisoned");
                            if shared.is_none() {
                                *shared = Some(error);
                            }
                            break;
                        }
                    }
                }
                if !local.is_empty() {
                    collected
                        .lock()
                        .expect("frontier results mutex poisoned")
                        .extend(local);
                }
            });
        }
    });

    if let Some(error) = first_error
        .into_inner()
        .expect("frontier error mutex poisoned")
    {
        return Err(error);
    }
    if is_cancelled(cancel) {
        return Err("search task cancelled".to_string());
    }

    Ok(collected
        .into_inner()
        .expect("frontier results mutex poisoned"))
}

fn evaluate_prefix(
    args: &crate::Args,
    catalog: &CandidateCatalog,
    start_offsets: &[f64],
    early_ticks: usize,
    atoms: &[crate::PrefixAtom],
    expansion: &PrefixExpansion,
    cancel: Option<&AtomicBool>,
) -> Result<PrefixEvaluation, String> {
    let prefix = Arc::new(crate::prefix_spec_from_indices(&expansion.indices, atoms));
    let mut evaluated = 0_usize;
    let mut kept = 0_usize;
    let mut candidates = Vec::new();
    let mut frontier_best: Option<FrontierState> = None;
    let early_cadence_min_start = early_cadence_min_start_tick(args);

    for (cycle_index, cycle) in catalog.cycles.iter().enumerate() {
        let layout = crate::Layout::new(&prefix.cells, &cycle.cells);
        for &start_offset in start_offsets {
            for &entity_id_mod4 in &args.entity_id_mods {
                for &initial_tick_count in &args.initial_tick_counts {
                    if is_cancelled(cancel) {
                        return Err("search task cancelled".to_string());
                    }
                    let config = crate::SimConfig {
                        ticks: early_ticks,
                        start_x: start_offset,
                        start_y: args.start_y,
                        start_vx: args.start_vx,
                        start_vy: args.start_vy,
                        entity_id_mod4,
                        initial_tick_count,
                        start_on_ground: Some(args.start_on_ground),
                    };
                    evaluated += 1;
                    let early_sim = crate::simulate(&layout, &config);
                    let Some(early) = crate::best_early_cadence(
                        &early_sim,
                        early_cadence_min_start,
                        args.cadence_pairs,
                        args.cadence_tolerance,
                    ) else {
                        continue;
                    };
                    let base_early_score = early_candidate_score(&early, prefix.cells.len());
                    let (transient_shape_mae16, transient_shape_mae24) =
                        transient_shape_metrics(args, start_offset, &early_sim);
                    let frontier_score = base_early_score
                        + transient_shape_penalty(transient_shape_mae16, transient_shape_mae24);

                    let frontier_state = FrontierState {
                        indices: expansion.indices.clone(),
                        length: prefix.cells.len(),
                        signature: prefix.signature.clone(),
                        frontier_score,
                        last_atom_index: expansion.last_atom_index,
                        cadence_start_tick: early.cadence_start_tick,
                        block_hit_rate: early.cadence_block_hit_rate,
                        within_tolerance_rate: early.cadence_within_tolerance_rate,
                        mean_signed_distance_error: early.cadence_mean_signed_distance_error,
                        transient_shape_mae16,
                        transient_shape_mae24,
                    };
                    if frontier_best
                        .as_ref()
                        .map(|current| frontier_state.frontier_score < current.frontier_score)
                        .unwrap_or(true)
                    {
                        frontier_best = Some(frontier_state);
                    }

                    if !early.cadence_pass
                        && (!args.keep_weak
                            || early.cadence_block_hit_rate < args.min_early_block_hit_rate)
                    {
                        continue;
                    }
                    let id_text = format!(
                        "{}|{}|start={}|id={}|tick={}",
                        prefix.label,
                        cycle.name,
                        start_offset,
                        entity_id_mod4,
                        initial_tick_count
                    );
                    candidates.push(SelectedCandidate {
                        id: crate::stable_id(&id_text),
                        early_score: base_early_score,
                        transient_shape_mae16,
                        transient_shape_mae24,
                        address: CandidateRef { cycle_index },
                        prefix_indices: expansion.indices.clone(),
                        prefix: Arc::clone(&prefix),
                        start_offset,
                        entity_id_mod4,
                        initial_tick_count,
                        cadence: early,
                    });
                    kept += 1;
                }
            }
        }
    }

    Ok(PrefixEvaluation {
        frontier: frontier_best,
        evaluated,
        kept,
        candidates,
    })
}

fn early_cadence_min_start_tick(args: &crate::Args) -> usize {
    if !args.start_on_ground && args.start_vx >= 0.75 {
        1
    } else {
        5
    }
}

fn prune_frontier(states: Vec<FrontierState>, args: &crate::Args) -> Vec<FrontierState> {
    let mut ordered = states;
    ordered.sort_by(|left, right| {
        left.frontier_score
            .total_cmp(&right.frontier_score)
            .then_with(|| left.signature.cmp(&right.signature))
            .then_with(|| left.indices.cmp(&right.indices))
    });

    let mut by_bucket = HashMap::<String, usize>::new();
    let mut by_signature = HashMap::<String, usize>::new();
    let mut kept = Vec::with_capacity(args.beam_width.min(ordered.len()));

    for state in ordered {
        let bucket_key = frontier_bucket_key(&state);
        let bucket_entry = by_bucket.entry(bucket_key).or_default();
        if *bucket_entry >= args.bucket_keep {
            continue;
        }
        let signature_entry = by_signature.entry(state.signature.clone()).or_default();
        if *signature_entry >= args.frontier_structure_keep {
            continue;
        }
        *bucket_entry += 1;
        *signature_entry += 1;
        kept.push(state);
        if kept.len() >= args.beam_width {
            break;
        }
    }

    kept
}

fn frontier_bucket_key(state: &FrontierState) -> String {
    let last_atom = state
        .last_atom_index
        .map(|value| value.to_string())
        .unwrap_or_else(|| "root".to_string());
    let hit_bucket = (state.block_hit_rate * 4.0).floor() as i32;
    let tol_bucket = (state.within_tolerance_rate * 4.0).floor() as i32;
    let signed_bucket = (state.mean_signed_distance_error / 0.05).round() as i32;
    let transient_bucket = state
        .transient_shape_mae16
        .map(|value| (value / 0.005).round() as i32)
        .unwrap_or(-1);
    let transient_bucket_long = state
        .transient_shape_mae24
        .map(|value| (value / 0.0075).round() as i32)
        .unwrap_or(-1);
    format!(
        "len={}|atom={}|tick={}|hit={}|tol={}|signed={}|transient={}|transientLong={}",
        state.length,
        last_atom,
        state.cadence_start_tick.min(5),
        hit_bucket.clamp(0, 4),
        tol_bucket.clamp(0, 4),
        signed_bucket.clamp(-8, 8),
        transient_bucket.clamp(-1, 12),
        transient_bucket_long.clamp(-1, 12),
    )
}

fn compare_selected_candidates(
    left: &SelectedCandidate,
    right: &SelectedCandidate,
) -> std::cmp::Ordering {
    left
        .transient_shape_mae16
        .unwrap_or(f64::INFINITY)
        .total_cmp(&right.transient_shape_mae16.unwrap_or(f64::INFINITY))
        .then_with(|| {
            left.transient_shape_mae24
                .unwrap_or(f64::INFINITY)
                .total_cmp(&right.transient_shape_mae24.unwrap_or(f64::INFINITY))
        })
        .then_with(|| left.early_score
        .total_cmp(&right.early_score)
        )
        .then_with(|| compare_prefix_visit_order(&left.prefix_indices, &right.prefix_indices))
        .then_with(|| left.address.cycle_index.cmp(&right.address.cycle_index))
        .then_with(|| left.start_offset.total_cmp(&right.start_offset))
        .then_with(|| left.entity_id_mod4.cmp(&right.entity_id_mod4))
        .then_with(|| left.initial_tick_count.cmp(&right.initial_tick_count))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_prefix_visit_order(left: &[usize], right: &[usize]) -> std::cmp::Ordering {
    let shared = left.len().min(right.len());
    for index in 0..shared {
        if left[index] != right[index] {
            return right[index].cmp(&left[index]);
        }
    }
    left.len().cmp(&right.len())
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}
