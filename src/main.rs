use egraph_builder::{saturate_dag, saturate_dag_local, saturate_egraph_local};
use optimizer::extract_md_prune;
use egg::Id;
use std::fs::File;
use std::io::Write;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = r#"
    n0 = a
    n1 = b
    n2 = c
    n3 = n0 + n1
    n4 = n0 * n2
    n5 = n1 * n2
    n6 = n2 * n3
    n7 = n4 + n5
    n8 = n6 + n7
    outputs = n8
    "#;

    // Temporary local scope for testing RunnerLocal.
    // These will later be generated from the raw EGraph.
    let local_scope = vec![
        Id::from(0),
        Id::from(1),
        Id::from(2),
    ];

    println!("Input DAG:");
    println!("{program}");

    println!("Local scope: {:?}", local_scope);

    let saturated = saturate_dag_local(program, local_scope)
        .map_err(|error| std::io::Error::other(
            format!("Local E-graph saturation failed: {error}")
        ))?;
    
    let eclass_count = saturated.egraph.number_of_classes();
    let enode_count = saturated.egraph.total_size();

    println!("E-classes : {eclass_count}");
    println!("E-nodes   : {enode_count}");

    let mut file = File::create("saturation1.dot")?;
    write!(file, "{}", saturated.egraph.dot())?;

    println!("EGraph DOT written to saturation1.dot");
/**
    let pruned = extract_md_prune(&saturated);

    let mut file = File::create("pruning.dot")?;
    write!(file, "{}", pruned.dot())?;

    println!("EGraph DOT written to pruning.dot");

    let local_scope = vec![
        Id::from(3),
        Id::from(6),
    ];

    println!("Input DAG:");
    println!("{program}");

    println!("Local scope: {:?}", local_scope);

    let saturated = saturate_egraph_local(pruned, local_scope);
    let eclass_count = saturated.number_of_classes();
    let enode_count = saturated.total_size();

    println!("E-classes : {eclass_count}");
    println!("E-nodes   : {enode_count}");

    let mut file = File::create("saturation2.dot")?;
    write!(file, "{}", saturated.dot())?;
**/
    Ok(())
}

