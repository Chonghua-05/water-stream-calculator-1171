use super::{
    CandidateCatalog, SelectedCandidate, SelectedCandidateSet, VerifiedCandidate, VerifiedCandidateSet,
};

pub(crate) struct ViewerRunReadyPayload {
    pub(crate) long_verified: usize,
    pub(crate) results: Vec<crate::ResultRow>,
}

pub(crate) fn verification_queue(
    args: &crate::Args,
    ranked_early_candidates: &[SelectedCandidate],
) -> Vec<SelectedCandidate> {
    let early_limited: Vec<_> = if args.early_limit > 0 {
        ranked_early_candidates
            .iter()
            .take(args.early_limit)
            .cloned()
            .collect()
    } else {
        ranked_early_candidates.to_vec()
    };
    if args.long_limit > 0 {
        early_limited.into_iter().take(args.long_limit).collect()
    } else {
        early_limited
    }
}

pub(crate) fn selected_to_search_payload(
    args: &crate::Args,
    catalog: &CandidateCatalog,
    selected: SelectedCandidateSet,
) -> crate::SearchPayload {
    let limit = if args.early_limit > 0 {
        args.early_limit
    } else {
        selected.candidates.len()
    };
    let results = selected
        .candidates
        .iter()
        .take(limit)
        .map(|candidate| early_result_row(catalog, candidate))
        .collect();
    crate::SearchPayload {
        evaluated: selected.evaluated,
        early_kept: selected.kept,
        early_deduped: selected.candidates.len(),
        long_verified: 0,
        results,
    }
}

pub(crate) fn prepare_verified_for_import(
    catalog: &CandidateCatalog,
    verified: VerifiedCandidateSet,
) -> ViewerRunReadyPayload {
    let results = verified
        .candidates
        .into_iter()
        .map(|verified_candidate| verified_result_row(catalog, &verified_candidate))
        .collect();
    ViewerRunReadyPayload {
        long_verified: verified.long_verified,
        results,
    }
}

pub(crate) fn viewer_ready_to_search_payload(
    selected: &SelectedCandidateSet,
    viewer_ready: ViewerRunReadyPayload,
    early_deduped: usize,
) -> crate::SearchPayload {
    crate::SearchPayload {
        evaluated: selected.evaluated,
        early_kept: selected.kept,
        early_deduped,
        long_verified: viewer_ready.long_verified,
        results: viewer_ready.results,
    }
}

fn early_result_row(catalog: &CandidateCatalog, candidate: &SelectedCandidate) -> crate::ResultRow {
    let prefix = catalog.prefix(candidate);
    let cycle = catalog.cycle(candidate);
    crate::ResultRow {
        id: candidate.id.clone(),
        pass: if candidate.cadence.cadence_pass {
            "early".to_string()
        } else {
            "weak-early".to_string()
        },
        score: candidate.early_score,
        early_score: candidate.early_score,
        prefix_label: prefix.label.clone(),
        prefix_length: prefix.cells.len(),
        backbone: cycle.name.clone(),
        proven: cycle.proven,
        start_offset: candidate.start_offset,
        entity_id_mod4: candidate.entity_id_mod4,
        initial_tick_count: candidate.initial_tick_count,
        period: cycle.cells.len(),
        cadence_start_tick: candidate.cadence.cadence_start_tick,
        cadence_pairs: candidate.cadence.cadence_pairs,
        cadence_mean_abs_distance_error: candidate.cadence.cadence_mean_abs_distance_error,
        cadence_mean_signed_distance_error: candidate.cadence.cadence_mean_signed_distance_error,
        cadence_max_abs_distance_error: candidate.cadence.cadence_max_abs_distance_error,
        cadence_block_hit_rate: candidate.cadence.cadence_block_hit_rate,
        cadence_within_tolerance_rate: candidate.cadence.cadence_within_tolerance_rate,
        cadence_pass: candidate.cadence.cadence_pass,
        cadence_samples: candidate.cadence.cadence_samples.clone(),
        full_cadence_start_tick: None,
        full_cadence_pairs: None,
        full_cadence_mean_abs_distance_error: None,
        full_cadence_mean_signed_distance_error: None,
        full_cadence_max_abs_distance_error: None,
        full_cadence_block_hit_rate: None,
        full_cadence_within_tolerance_rate: None,
        full_cadence_longest_hit_run: None,
        full_cadence_average_speed: None,
        full_cadence_distance: None,
        full_cadence_first_miss: None,
        full_cadence_min_hit_margin: None,
        full_cadence_mean_hit_margin: None,
        full_cadence_min_endpoint_boundary_margin: None,
        full_cadence_mean_endpoint_boundary_margin: None,
        full_cadence_samples: None,
        long_window_start_tick: None,
        long_average_vx: None,
        long_mean_vx_error: None,
        long_std_vx: None,
        long_average_distance_vx: None,
        suffix_average_vx: None,
        suffix_mean_vx_error: None,
        suffix_std_vx: None,
        suffix_average_distance_vx: None,
        first_ticks: None,
        prefix_cells: layout_cells_description(&prefix.cells),
        cycle_cells: layout_cells_description(&cycle.cells),
        note: cycle.note.clone(),
    }
}

fn verified_result_row(
    catalog: &CandidateCatalog,
    candidate: &VerifiedCandidate,
) -> crate::ResultRow {
    let prefix = catalog.prefix(&candidate.selected);
    let cycle = catalog.cycle(&candidate.selected);
    crate::ResultRow {
        id: candidate.selected.id.clone(),
        pass: candidate.pass.to_string(),
        score: candidate.score,
        early_score: candidate.selected.early_score,
        prefix_label: prefix.label.clone(),
        prefix_length: prefix.cells.len(),
        backbone: cycle.name.clone(),
        proven: cycle.proven,
        start_offset: candidate.selected.start_offset,
        entity_id_mod4: candidate.selected.entity_id_mod4,
        initial_tick_count: candidate.selected.initial_tick_count,
        period: cycle.cells.len(),
        cadence_start_tick: candidate.selected.cadence.cadence_start_tick,
        cadence_pairs: candidate.selected.cadence.cadence_pairs,
        cadence_mean_abs_distance_error: candidate
            .selected
            .cadence
            .cadence_mean_abs_distance_error,
        cadence_mean_signed_distance_error: candidate
            .selected
            .cadence
            .cadence_mean_signed_distance_error,
        cadence_max_abs_distance_error: candidate.selected.cadence.cadence_max_abs_distance_error,
        cadence_block_hit_rate: candidate.selected.cadence.cadence_block_hit_rate,
        cadence_within_tolerance_rate: candidate.selected.cadence.cadence_within_tolerance_rate,
        cadence_pass: candidate.selected.cadence.cadence_pass,
        cadence_samples: candidate.selected.cadence.cadence_samples.clone(),
        full_cadence_start_tick: Some(candidate.full_cadence.full_cadence_start_tick),
        full_cadence_pairs: Some(candidate.full_cadence.full_cadence_pairs),
        full_cadence_mean_abs_distance_error: Some(
            candidate.full_cadence.full_cadence_mean_abs_distance_error,
        ),
        full_cadence_mean_signed_distance_error: Some(
            candidate.full_cadence.full_cadence_mean_signed_distance_error,
        ),
        full_cadence_max_abs_distance_error: Some(
            candidate.full_cadence.full_cadence_max_abs_distance_error,
        ),
        full_cadence_block_hit_rate: Some(candidate.full_cadence.full_cadence_block_hit_rate),
        full_cadence_within_tolerance_rate: Some(
            candidate.full_cadence.full_cadence_within_tolerance_rate,
        ),
        full_cadence_longest_hit_run: Some(candidate.full_cadence.full_cadence_longest_hit_run),
        full_cadence_average_speed: Some(candidate.full_cadence.full_cadence_average_speed),
        full_cadence_distance: Some(candidate.full_cadence.full_cadence_distance),
        full_cadence_first_miss: candidate.full_cadence.full_cadence_first_miss.clone(),
        full_cadence_min_hit_margin: Some(candidate.full_cadence.full_cadence_min_hit_margin),
        full_cadence_mean_hit_margin: Some(candidate.full_cadence.full_cadence_mean_hit_margin),
        full_cadence_min_endpoint_boundary_margin: Some(
            candidate.full_cadence.full_cadence_min_endpoint_boundary_margin,
        ),
        full_cadence_mean_endpoint_boundary_margin: Some(
            candidate
                .full_cadence
                .full_cadence_mean_endpoint_boundary_margin,
        ),
        full_cadence_samples: Some(candidate.full_cadence.full_cadence_samples.clone()),
        long_window_start_tick: candidate.long_window.long_window_start_tick,
        long_average_vx: Some(candidate.long_window.average_vx),
        long_mean_vx_error: Some(candidate.long_window.mean_vx_error),
        long_std_vx: Some(candidate.long_window.std_vx),
        long_average_distance_vx: Some(candidate.long_window.average_distance_vx),
        suffix_average_vx: candidate.suffix_window.as_ref().map(|value| value.average_vx),
        suffix_mean_vx_error: candidate
            .suffix_window
            .as_ref()
            .map(|value| value.mean_vx_error),
        suffix_std_vx: candidate.suffix_window.as_ref().map(|value| value.std_vx),
        suffix_average_distance_vx: candidate
            .suffix_window
            .as_ref()
            .map(|value| value.average_distance_vx),
        first_ticks: Some(first_ticks(&candidate.simulation)),
        prefix_cells: layout_cells_description(&prefix.cells),
        cycle_cells: layout_cells_description(&cycle.cells),
        note: cycle.note.clone(),
    }
}

fn layout_cells_description(cells: &[crate::Cell]) -> Vec<crate::CellDescription> {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| crate::CellDescription {
            index,
            surface: cell.surface,
            flow: cell.flow,
            derived_flow_hint: (cell.amount == 0).then_some(0),
            amount: cell.amount,
            floor: cell.floor.as_str().to_string(),
            code: cell.code(),
        })
        .collect()
}

fn first_ticks(sim: &crate::Simulation) -> Vec<crate::FirstTick> {
    (0..sim.xs.len().min(16))
        .map(|tick| crate::FirstTick {
            tick,
            x: sim.xs[tick],
            y: sim.ys[tick],
            vx: sim.vxs[tick],
            vy: sim.vys[tick],
            floor: sim.floors[tick].as_str().to_string(),
            on_ground: sim.on_grounds[tick] != 0,
        })
        .collect()
}
