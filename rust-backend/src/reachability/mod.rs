use std::sync::Arc;

pub(crate) mod import;
mod search;
mod verify;

#[derive(Clone, Debug)]
pub(crate) struct PrepareProgress {
    pub(crate) layer: usize,
    pub(crate) frontier_in: usize,
    pub(crate) processed_prefixes: usize,
    pub(crate) expanded_prefixes: usize,
    pub(crate) frontier_out: usize,
    pub(crate) evaluated_total: usize,
    pub(crate) kept_total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CandidateRef {
    pub(crate) cycle_index: usize,
}

#[derive(Clone)]
pub(crate) struct CandidateCatalog {
    pub(crate) cycles: Vec<crate::CycleSpec>,
}

impl CandidateCatalog {
    pub(crate) fn new(args: &crate::Args) -> Self {
        let cycles = if let Some(requested_names) = args.cycle_names.as_ref() {
            let all_cycles = crate::backbone_cycles();
            requested_names
                .iter()
                .filter_map(|name| all_cycles.iter().find(|cycle| cycle.name == *name).cloned())
                .collect()
        } else {
            crate::backbone_cycles()
        };
        Self { cycles }
    }

    pub(crate) fn prefix<'a>(&self, candidate: &'a SelectedCandidate) -> &'a crate::PrefixSpec {
        candidate.prefix.as_ref()
    }

    pub(crate) fn cycle<'a>(&'a self, candidate: &SelectedCandidate) -> &'a crate::CycleSpec {
        &self.cycles[candidate.address.cycle_index]
    }

    pub(crate) fn layout(&self, candidate: &SelectedCandidate) -> crate::Layout {
        crate::Layout::new(&self.prefix(candidate).cells, &self.cycle(candidate).cells)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SelectedCandidate {
    pub(crate) id: String,
    pub(crate) early_score: f64,
    pub(crate) transient_shape_mae16: Option<f64>,
    pub(crate) transient_shape_mae24: Option<f64>,
    pub(crate) address: CandidateRef,
    pub(crate) prefix_indices: Vec<usize>,
    pub(crate) prefix: Arc<crate::PrefixSpec>,
    pub(crate) start_offset: f64,
    pub(crate) entity_id_mod4: usize,
    pub(crate) initial_tick_count: usize,
    pub(crate) cadence: crate::EarlyCadence,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SelectedCandidateSet {
    pub(crate) evaluated: usize,
    pub(crate) kept: usize,
    pub(crate) candidates: Vec<SelectedCandidate>,
}

#[derive(Clone)]
pub(crate) struct VerifiedCandidate {
    pub(crate) selected: SelectedCandidate,
    pub(crate) pass: &'static str,
    pub(crate) score: f64,
    pub(crate) full_cadence: crate::FullCadence,
    pub(crate) long_window: crate::WindowMetrics,
    pub(crate) suffix_window: Option<crate::WindowMetrics>,
    pub(crate) simulation: crate::Simulation,
}

#[derive(Clone, Default)]
pub(crate) struct VerifiedCandidateSet {
    pub(crate) long_verified: usize,
    pub(crate) candidates: Vec<VerifiedCandidate>,
}

#[derive(Clone)]
pub(crate) struct PreparedSearch {
    pub(crate) catalog: CandidateCatalog,
    pub(crate) selected: SelectedCandidateSet,
    pub(crate) ranked_early_candidates: Vec<SelectedCandidate>,
}

pub(crate) fn prepare_cancelable(
    args: &crate::Args,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    progress: Option<&mut dyn FnMut(PrepareProgress)>,
) -> Result<PreparedSearch, String> {
    search::prepare_cancelable(args, cancel, progress)
}

pub(crate) fn run(args: &crate::Args) -> crate::SearchPayload {
    search::run(args)
}
