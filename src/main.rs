use egraph_builder::{saturate_dag, saturate_dag_local};
use egg::Id;

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
    n4 = n3 + n2
    n5 = n1 * n2
    n6 = n0 + n5
    n7 = n4 * n6
    
    outputs = n7
    "#;

    // Temporary local scope for testing RunnerLocal.
    // These will later be generated from the raw EGraph.
    let local_scope = vec![
        Id::from(3),
        Id::from(4),
        Id::from(5),
        Id::from(6),
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

    let saturated = saturate_dag(program)
    .map_err(|error| std::io::Error::other(
        format!("Local E-graph saturation failed: {error}")
    ))?;

    let eclass_count = saturated.egraph.number_of_classes();
    let enode_count = saturated.egraph.total_size();

    println!("E-classes : {eclass_count}");
    println!("E-nodes   : {enode_count}");

    Ok(())
}

