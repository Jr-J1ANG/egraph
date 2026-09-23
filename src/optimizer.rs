use crate::cost::{GlobalMcCost, MdCost, MinMdTreeCost};
use crate::dag::{Dag, DagStats};
use crate::egraph_builder::SaturatedEGraph;
use crate::lang::FheLang;
use crate::md_mc_extractor::MdMcExtractor;

use egg::{Extractor, LpExtractor, RecExpr};
use good_lp::coin_cbc;

/// Keep the old public path available for printers and other callers.
pub use crate::egraph_builder::InputKind;
/// Compatibility re-export: existing printer code can keep importing this
/// result type from `optimizer`.
pub use crate::evaluator::DagEvaluation;

pub struct MdOptimizationResult {
    pub input: InputKind,
    pub best: RecExpr<FheLang>,
    pub md_cost: MdCost,
    pub dag: Dag,
    pub dag_stats: DagStats,
    pub iterations: usize,
    pub stop_reason: String,
}

pub struct GlobalMcOptimizationResult {
    pub input: InputKind,
    pub best: RecExpr<FheLang>,
    pub dag: Dag,
    pub dag_stats: DagStats,
    pub iterations: usize,
    pub stop_reason: String,
}

/// Globally MC-minimal extraction subject to a caller-provided MD upper bound.
///
/// Unlike `extract_md`, this solves a MIP over the whole extracted DAG.  The
/// result retains the same metadata layout as the other optimizer outputs, so
/// printing and Stage-3 enumeration do not need to know about the extractor's
/// internal MIP representation.
pub struct MdMcOptimizationResult {
    pub input: InputKind,

    /// Requested feasibility bound: extracted DAG must satisfy MD <= md_limit.
    pub md_limit: usize,

    /// Globally selected multi-output expression.
    pub best: RecExpr<FheLang>,
    pub dag: Dag,
    pub dag_stats: DagStats,

    /// Objective value reported by the MIP. This is checked against
    /// `dag_stats.mc` by `MdMcExtractor` before being returned.
    pub optimal_mc: usize,

    pub iterations: usize,
    pub stop_reason: String,
}

/// Extract a multiplication-depth-minimal candidate from an already saturated
/// e-graph. This does not parse input, construct an e-graph, or run rewrites.
pub fn extract_md(saturated: &SaturatedEGraph) -> MdOptimizationResult {
    let extractor = Extractor::new(&saturated.egraph, MinMdTreeCost);
    let (md_cost, best) = extractor.find_best(saturated.root);

    let dag = Dag::from_recexpr(&best);
    let dag_stats = dag.stats();

    MdOptimizationResult {
        input: saturated.input.clone(),
        best,
        md_cost,
        dag,
        dag_stats,
        iterations: saturated.iterations,
        stop_reason: saturated.stop_reason.clone(),
    }
}

/// Maximum time given specifically to the ILP solve.
///
/// This is independent of the equality-saturation time limit.
const LP_EXTRACTION_TIMEOUT_SECS: f64 = 100.0;

/// Extract one globally MC-minimal multi-output DAG from an already saturated
/// e-graph. The virtual `Outputs(...)` root makes all program outputs part of
/// the same ILP model, allowing global sharing across outputs.
pub fn extract_global_mc(saturated: &SaturatedEGraph) -> GlobalMcOptimizationResult {
    let mut extractor = LpExtractor::new(&saturated.egraph, GlobalMcCost);
    let best = extractor.solve_with_timeout(saturated.root, coin_cbc, LP_EXTRACTION_TIMEOUT_SECS);
    drop(extractor);

    let dag = Dag::from_recexpr(&best);
    let dag_stats = dag.stats();

    GlobalMcOptimizationResult {
        input: saturated.input.clone(),
        best,
        dag,
        dag_stats,
        iterations: saturated.iterations,
        stop_reason: saturated.stop_reason.clone(),
    }
}

///minimise MC  subject to MD <= md_limit.
/// No `MD_min` is used here. If the bound is impossible, the underlying MIP
/// returns an error; the caller can therefore test infeasible limits directly.
pub fn extract_mc_under_md(
    saturated: &SaturatedEGraph,
    md_limit: usize,
) -> Result<MdMcOptimizationResult, String> {
    let (best, optimal_mc) = MdMcExtractor::from_saturated(saturated).solve(md_limit)?;

    // Extractor counts for MIP solving, while optimizer for materialize verification.
    let dag = Dag::from_recexpr(&best);
    let dag_stats = dag.stats();

    if dag_stats.md > md_limit {
        return Err(format!(
            "Internal validation failed: extracted DAG has MD={} > requested limit {}",
            dag_stats.md, md_limit
        ));
    }
    if dag_stats.mc != optimal_mc {
        return Err(format!(
            "Internal validation failed: MIP MC={} but materialized DAG MC={}",
            optimal_mc, dag_stats.mc
        ));
    }
    Ok(MdMcOptimizationResult {
        input: saturated.input.clone(),
        md_limit,
        best,
        dag,
        dag_stats,
        optimal_mc,
        iterations: saturated.iterations,
        stop_reason: saturated.stop_reason.clone(),
    })
}

