use egg::Id;
use std::fs;
use std::io::{self, Write};

use egraph_builder::{saturate_dag_local, saturate_egraph_local};
use optimizer::{extract_global_mc_prune, extract_md_prune};

mod cost;
mod dag;
mod dag_parser;
mod egraph_builder;
mod evaluator;
mod expr_parser;
mod lang;
mod md_mc_extractor;
mod optimizer;
mod print;
mod rules;

// Number of consecutive unparticipated nodes assigned to each local scope.
// Change this constant to experiment with different group sizes.
const LOCAL_SCOPE_SIZE: usize = 8;

// dag.txt is read from the current working directory.
const INPUT_DAG_PATH: &str = "dag.txt";

fn write_dag_text(path: &str, dag: &dag::Dag) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    for (id, node) in dag.nodes().iter().enumerate() {
        use dag::DagKey;
        match node {
            DagKey::Num(n) => writeln!(file, "n{id} = {n}")?,
            DagKey::Symbol(s) => writeln!(file, "n{id} = {s}")?,
            DagKey::Add(a, b) => writeln!(file, "n{id} = n{a} + n{b}")?,
            DagKey::Mul(a, b) => writeln!(file, "n{id} = n{a} * n{b}")?,
            DagKey::Neg(a) => writeln!(file, "n{id} = - n{a}")?,
        }
    }
    // The DAG IR may carry one or multiple roots. This benchmark uses one,
    // but serialize all roots if the IR exposes the multi-output interface.
    let outputs = dag.roots()
        .iter()
        .map(|root| format!("n{root}"))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(file, "outputs = {outputs}")?;
    Ok(())
}

fn choose_scope(unparticipated: &[Id], count: usize) -> Vec<Id> {
    unparticipated.iter().take(count.max(1)).copied().collect()
}

fn wrap_pruned_egraph(
    egraph: egg::EGraph<lang::FheLang, ()>,
    root: Id,
    node_count: usize,
) -> egraph_builder::SaturatedEGraph {
    egraph_builder::SaturatedEGraph {
        egraph,
        root,
        input: egraph_builder::InputKind::Dag {
            node_count,
            output_count: 1,
        },
        iterations: 0,
        stop_reason: "local saturation on pruned e-graph".to_string(),
    }
}

fn run_md(program: &str, node_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n========== Local MD optimization ==========");
    let mut unparticipated: Vec<Id> = (0..node_count).map(Id::from).collect();
    let mut current_scope = choose_scope(&unparticipated, LOCAL_SCOPE_SIZE);

    // The first local run constructs the original e-graph. Subsequent rounds
    // continue from the previous round's pruned e-graph.
    let mut saturated = saturate_dag_local(program, current_scope.clone())
        .map_err(|e| io::Error::other(format!("Initial local saturation failed: {e}")))?;

    let mut round = 1usize;
    loop {
        let (pruned, _root, next_unparticipated) =
            extract_md_prune(&saturated, current_scope.clone(), unparticipated.clone());
        unparticipated = next_unparticipated;

        // Report the selected DAG's global MD/MC after every MD round, not just
        // the e-graph size. Clone the pruned graph because the next round reuses it.
        let round_egraph = saturate_egraph_local(pruned.clone(), Vec::new());
        let round_saturated = wrap_pruned_egraph(round_egraph, _root, node_count);
        let round_result = optimizer::extract_md(&round_saturated);
        println!(
            "[MD] round {round}: scope={}, remaining={}, eclasses={}, enodes={}, MD={}, MC={}, MD^2*MC={}, nodes={}",
            current_scope.len(),
            unparticipated.len(),
            pruned.number_of_classes(),
            pruned.total_size(),
            round_result.dag_stats.md,
            round_result.dag_stats.mc,
            round_result.dag_stats.md * round_result.dag_stats.md * round_result.dag_stats.mc,
            round_result.dag_stats.total_nodes
        );

        if unparticipated.is_empty() {
            // Re-extract the final selected implementation for serialization.
            let final_egraph = saturate_egraph_local(pruned, Vec::new());
            let final_saturated = wrap_pruned_egraph(final_egraph, _root, node_count);
            let result = optimizer::extract_md(&final_saturated);
            println!(
                "[MD] final: MD={}, MC={}, nodes={}",
                result.dag_stats.md, result.dag_stats.mc, result.dag_stats.total_nodes
            );
            write_dag_text("dag_md.txt", &result.dag)?;
            break;
        }

        current_scope = choose_scope(&unparticipated, LOCAL_SCOPE_SIZE);
        let next_egraph = saturate_egraph_local(pruned, current_scope.clone());
        saturated = wrap_pruned_egraph(next_egraph, _root, node_count);
        round += 1;
    }
    Ok(())
}

fn run_mc(program: &str, node_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n========== Local MC optimization ==========");
    let mut unparticipated: Vec<Id> = (0..node_count).map(Id::from).collect();
    let mut current_scope = choose_scope(&unparticipated, LOCAL_SCOPE_SIZE);

    let mut saturated = saturate_dag_local(program, current_scope.clone())
        .map_err(|e| io::Error::other(format!("Initial local saturation failed: {e}")))?;

    let mut round = 1usize;
    loop {
        let (pruned, _root, next_unparticipated) =
            extract_global_mc_prune(&saturated, current_scope.clone(), unparticipated.clone());
        unparticipated = next_unparticipated;

        println!(
            "[MC] round {round}: scope={}, remaining={}, eclasses={}, enodes={}",
            current_scope.len(),
            unparticipated.len(),
            pruned.number_of_classes(),
            pruned.total_size()
        );

        if unparticipated.is_empty() {
            let final_egraph = saturate_egraph_local(pruned, Vec::new());
            let final_saturated = wrap_pruned_egraph(final_egraph, _root, node_count);
            let result = optimizer::extract_global_mc(&final_saturated);
            println!(
                "[MC] final: MD={}, MC={}, nodes={}",
                result.dag_stats.md, result.dag_stats.mc, result.dag_stats.total_nodes
            );
            write_dag_text("dag_mc.txt", &result.dag)?;
            break;
        }

        current_scope = choose_scope(&unparticipated, LOCAL_SCOPE_SIZE);
        let next_egraph = saturate_egraph_local(pruned, current_scope.clone());
        saturated = wrap_pruned_egraph(next_egraph, _root, node_count);
        round += 1;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = fs::read_to_string(INPUT_DAG_PATH)?;
    let parsed = dag_parser::ProgramDag::parse(&program)
        .map_err(|e| io::Error::other(format!("Failed to parse {INPUT_DAG_PATH}: {e}")))?;
    let node_count = parsed.node_count();

    println!("Input DAG: {INPUT_DAG_PATH}");
    println!("Arithmetic/input nodes: {node_count}");
    println!("Local scope size: {LOCAL_SCOPE_SIZE}");

    // Measure the original DAG as written, before any rewriting or extraction.
    let original = evaluator::evaluate_dag(&program)
        .map_err(|e| io::Error::other(format!("Failed to evaluate original DAG: {e}")))?;
    println!(
        "Original DAG stats: MD={}, MC={}, MD^2*MC={}, reachable_nodes={}",
        original.stats.md,
        original.stats.mc,
        original.stats.fhe_cost,
        original.stats.total_nodes
    );
    println!("Original DAG:\n{program}");

    // These are deliberately independent runs. MC starts again from dag.txt,
    // not from the MD-optimized output.
    run_md(&program, node_count)?;
    run_mc(&program, node_count)?;

    println!("\nSaved optimized DAGs: dag_md.txt and dag_mc.txt");
    Ok(())
}
