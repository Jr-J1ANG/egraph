use crate::dag_parser::ProgramDag;
use crate::lang::FheLang;
use crate::rules::rules;

use egg::{EGraph, Id, RecExpr, Runner, RunnerLocal};
use std::time::Duration;

/// Metadata about the original program.
///
/// It travels with the saturated e-graph so every extraction result can be
/// reported without reparsing the input.
#[derive(Debug, Clone)]
pub enum InputKind {
    Formula(String),
    Dag {
        node_count: usize,
        output_count: usize,
    },
}

/// One equality-saturation run, ready to be reused by any extractor.
///
/// The `Outputs(...)` virtual root represents all program outputs jointly.
/// Therefore every extractor sees the same multi-output optimization problem.
pub struct SaturatedEGraph {
    pub(crate) egraph: EGraph<FheLang, ()>,
    pub(crate) root: Id,
    pub(crate) input: InputKind,
    pub(crate) iterations: usize,
    pub(crate) stop_reason: String,
}

const ITER_LIMIT: usize = 100;
const NODE_LIMIT: usize = 1000_000;
const TIME_LIMIT_SECS: u64 = 2;

fn saturate(egraph: EGraph<FheLang, ()>, root_hint: Id, input: InputKind) -> SaturatedEGraph {
    let runner = Runner::default()
        .with_egraph(egraph)
        .with_iter_limit(ITER_LIMIT)
        .with_node_limit(NODE_LIMIT)
        .with_time_limit(Duration::from_secs(TIME_LIMIT_SECS))
        .run(&rules());

    // Saturation may merge the original root into another e-class.
    let root = runner.egraph.find(root_hint);

    SaturatedEGraph {
        egraph: runner.egraph,
        root,
        input,
        iterations: runner.iterations.len(),
        stop_reason: format!("{:?}", runner.stop_reason),
    }
}

fn saturate_local(egraph: EGraph<FheLang, ()>, root_hint: Id, input: InputKind, local_scope: Vec<Id>) -> SaturatedEGraph {
    let runner = RunnerLocal::default()
        .with_egraph(egraph)
        .with_iter_limit(ITER_LIMIT)
        .with_node_limit(NODE_LIMIT)
        .with_time_limit(Duration::from_secs(TIME_LIMIT_SECS))
        .with_local_scope(local_scope)
        .run(&rules());

    // Saturation may merge the original root into another e-class.
    let root = runner.egraph.find(root_hint);

    SaturatedEGraph {
        egraph: runner.egraph,
        root,
        input,
        iterations: runner.iterations.len(),
        stop_reason: format!("{:?}", runner.stop_reason),
    }
}

/// Parse a single-output S-expression, add a virtual `outputs(...)` root,
/// then run equality saturation exactly once.
pub fn saturate_formula(input: &str) -> Result<SaturatedEGraph, String> {
    let expr: RecExpr<FheLang> = input
        .parse()
        .map_err(|error| format!("Invalid S-expression input: {error:?}"))?;

    let mut egraph = EGraph::<FheLang, ()>::default();
    let formula_root = egraph.add_expr(&expr);
    let outputs_root = egraph.add(FheLang::Outputs(vec![formula_root].into_boxed_slice()));
    egraph.rebuild();

    Ok(saturate(
        egraph,
        outputs_root,
        InputKind::Formula(expr.to_string()),
    ))
}

/// Parse a textual multi-output DAG, construct its e-graph with a virtual
/// `outputs(...)` root, then run equality saturation exactly once.
pub fn saturate_dag(input: &str) -> Result<SaturatedEGraph, String> {
    let program = ProgramDag::parse(input)?;
    let input_kind = InputKind::Dag {
        node_count: program.node_count(),
        output_count: program.output_count(),
    };

    let (egraph, outputs_root) = program.to_egraph()?;
    Ok(saturate(egraph, outputs_root, input_kind))
}
