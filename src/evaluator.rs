use crate::dag::{Dag, DagStats};
use crate::dag_parser::ProgramDag;
use crate::lang::FheLang;
use egg::RecExpr;

pub struct DagEvaluation {
    pub input: String,
    pub dag: Dag,
    pub stats: DagStats,
}

/// Evaluate a formula as a one-output DAG without e-graph construction,
/// rewriting, saturation, or extraction.
pub fn evaluate_formula_dag(input: &str) -> Result<DagEvaluation, String> {
    let expr: RecExpr<FheLang> = input
        .parse()
        .map_err(|error| format!("Invalid S-expression input: {error:?}"))?;

    let dag = Dag::from_recexpr(&expr);
    let stats = dag.stats();

    Ok(DagEvaluation {
        input: expr.to_string(),
        dag,
        stats,
    })
}

/// Evaluate the textual DAG exactly as written, preserving its explicit
/// sharing and without constructing an e-graph.
pub fn evaluate_dag(input: &str) -> Result<DagEvaluation, String> {
    let program = ProgramDag::parse(input)?;
    let dag = program.to_dag()?;
    let stats = dag.stats();

    Ok(DagEvaluation {
        input: input.trim().to_string(),
        dag,
        stats,
    })
}

