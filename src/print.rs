use crate::dag::{Dag, DagKey};
use crate::optimizer::{
    DagEvaluation, GlobalMcOptimizationResult, InputKind, MdMcOptimizationResult,
    MdOptimizationResult,
};

use crate::egraph_builder::SaturatedEGraph;
use std::fs;
use std::io;
use std::path::Path;

fn print_input_summary(input: &InputKind) {
    match input {
        InputKind::Formula(expression) => println!("Input formula: {expression}"),
        InputKind::Dag {
            node_count,
            output_count,
        } => println!("Input DAG nodes: {node_count}, outputs: {output_count}"),
    }
}

fn print_saturation_summary(iterations: usize, stop_reason: &str) {
    println!("Iterations = {iterations}, stop reason = {stop_reason}");
}

pub fn print_md_optimization_result(result: &MdOptimizationResult) {
    println!("=== MD Optimization result ===");
    print_input_summary(&result.input);
    println!("Best expression: {}", result.best);

    println!(
        "Tree depth objective: MD = {}, tree MC = {}, AST size = {}",
        result.md_cost.md, result.md_cost.mc, result.md_cost.ast_size
    );

    println!(
        "Extracted DAG cost: MC = {}, MD = {}, MC × MD² = {}, unique nodes = {}",
        result.dag_stats.mc,
        result.dag_stats.md,
        result.dag_stats.fhe_cost,
        result.dag_stats.total_nodes
    );

    print_saturation_summary(result.iterations, &result.stop_reason);
    print_dag_nodes(&result.dag);
    println!();
}

pub fn print_global_mc_optimization_result(result: &GlobalMcOptimizationResult) {
    println!("=== Global MC optimization result ===");
    print_input_summary(&result.input);
    println!("Best expression: {}", result.best);
    println!("Objective: minimise global unique multiplication count (MC)");

    println!(
        "Extracted DAG cost: MC = {}, MD = {}, MC × MD² = {}, unique nodes = {}",
        result.dag_stats.mc,
        result.dag_stats.md,
        result.dag_stats.fhe_cost,
        result.dag_stats.total_nodes,
    );

    print_saturation_summary(result.iterations, &result.stop_reason);
    print_dag_nodes(&result.dag);
    println!();
}

/// Print the result of the Stage-3 constrained global extraction.
///
/// `MD limit` is the requested upper bound; `actual MD` is recomputed from the
/// materialized DAG and can be strictly smaller than the limit.
pub fn print_md_mc_optimization_result(result: &MdMcOptimizationResult) {
    println!("=== MD-bounded Global MC optimization result ===");
    print_input_summary(&result.input);
    println!("Best expression: {}", result.best);
    println!(
        "Objective: minimise global unique MC subject to MD <= {}",
        result.md_limit
    );

    println!(
        "MIP objective: MC = {}; extracted DAG: MC = {}, actual MD = {}, MC × MD² = {}, unique nodes = {}",
        result.optimal_mc,
        result.dag_stats.mc,
        result.dag_stats.md,
        result.dag_stats.fhe_cost,
        result.dag_stats.total_nodes,
    );

    print_saturation_summary(result.iterations, &result.stop_reason);
    print_dag_nodes(&result.dag);
    println!();
}

pub fn print_dag_evaluation(result: &DagEvaluation) {
    println!("=== DAG evaluation ===");
    println!("Input:");
    println!("{}", result.input);

    println!(
        "DAG cost: MC = {}, MD = {}, MC × MD² = {}, unique nodes = {}",
        result.stats.mc, result.stats.md, result.stats.fhe_cost, result.stats.total_nodes
    );

    print_dag_nodes(&result.dag);
    println!();
}

fn print_dag_nodes(dag: &Dag) {
    println!("DAG nodes:");

    for (id, node) in dag.nodes().iter().enumerate() {
        match node {
            DagKey::Num(value) => println!("  n{id} = {value}"),
            DagKey::Symbol(name) => println!("  n{id} = {name}"),
            DagKey::Add(left, right) => println!("  n{id} = n{left} + n{right}"),
            DagKey::Mul(left, right) => println!("  n{id} = n{left} * n{right}"),
            DagKey::Neg(child) => println!("  n{id} = -n{child}"),
        }
    }

    let outputs = dag
        .roots()
        .iter()
        .map(|root| format!("n{root}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!("  outputs = {outputs}");
}

pub fn print_egraph_dot(saturated: &SaturatedEGraph, path: impl AsRef<Path>) -> io::Result<()> {
    let dot = saturated.egraph.dot();

    fs::write(path, dot.to_string())
}

