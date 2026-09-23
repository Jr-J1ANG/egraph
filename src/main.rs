use egraph_builder::saturate_dag;
use evaluator::evaluate_dag;
use optimizer::{extract_global_mc, extract_mc_under_md, extract_md};

use dag::{Dag, DagKey};

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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

/// Batch input directory. Every `.txt` file in this directory is processed in
/// lexicographic order, i.e. from `poly_d6_mask_0000001.txt` to
/// `poly_d6_mask_1111111.txt`.
const INPUT_DIR: &str = "benchmarks/test";

/// All generated DAGs and the final summary are written here. Keeping results
/// outside the input directory prevents a second run from treating outputs as
/// new benchmark inputs.
const OUTPUT_DIR: &str = "benchmarks/results/test";
const RESULT_FILE: &str = "benchmarks/results/test/result.txt";

fn dag_to_text(dag: &Dag) -> String {
    let mut text = String::new();

    for (id, node) in dag.nodes().iter().enumerate() {
        match node {
            DagKey::Num(value) => text.push_str(&format!("n{id} = {value}\n")),
            DagKey::Symbol(name) => text.push_str(&format!("n{id} = {name}\n")),
            DagKey::Add(left, right) => {
                text.push_str(&format!("n{id} = n{left} + n{right}\n"));
            }
            DagKey::Mul(left, right) => {
                text.push_str(&format!("n{id} = n{left} * n{right}\n"));
            }
            DagKey::Neg(child) => text.push_str(&format!("n{id} = - n{child}\n")),
        }
    }

    let outputs = dag
        .roots()
        .iter()
        .map(|root| format!("n{root}"))
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!("\noutputs = {outputs}\n"));

    text
}

fn write_dag(path: &Path, dag: &Dag) -> Result<(), Box<dyn Error>> {
    fs::write(path, dag_to_text(dag))?;
    Ok(())
}

fn benchmark_output_paths(
    input_path: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), Box<dyn Error>> {
    let name = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| io::Error::other("Invalid benchmark file name"))?;

    let output_dir = Path::new(OUTPUT_DIR);
    fs::create_dir_all(output_dir)?;

    Ok((
        output_dir.join(format!("{name}_original_dag.txt")),
        output_dir.join(format!("{name}_md_opt_dag.txt")),
        output_dir.join(format!("{name}_mc_opt_dag.txt")),
        output_dir.join(format!("{name}_mcmd_opt_dag.txt")),
    ))
}

/// Append one completed unit of progress immediately.
///
/// `flush` moves Rust's buffered bytes into the OS, and `sync_all` asks the OS
/// to persist them. Therefore completed stages remain visible in `result.txt`
/// even if a later benchmark or extraction is interrupted.
fn append_result(result_file: &mut fs::File, text: &str) -> Result<(), Box<dyn Error>> {
    result_file.write_all(text.as_bytes())?;
    result_file.flush()?;
    result_file.sync_all()?;
    Ok(())
}

fn benchmark_files(input_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();

    for entry in fs::read_dir(input_dir)? {
        let path = entry?.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            files.push(path);
        }
    }

    files.sort();

    if files.is_empty() {
        return Err(io::Error::other(format!(
            "No .txt benchmark files found in {}",
            input_dir.display()
        ))
        .into());
    }

    Ok(files)
}

/// Run all four measurements for one polynomial and return the exact report
/// block used both on the console and in the final summary file.
fn process_one(input_path: &Path, result_file: &mut fs::File) -> Result<String, Box<dyn Error>> {
    let name = input_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("Invalid benchmark file name"))?;

    let program = fs::read_to_string(input_path)?;
    let program = program.trim();
    if program.is_empty() {
        return Err(
            io::Error::other(format!("Benchmark file is empty: {}", input_path.display())).into(),
        );
    }

    let (original_path, md_path, mc_path, mcmd_path) = benchmark_output_paths(input_path)?;

    append_result(
        result_file,
        &format!(
            "============================================================\n             Benchmark : {name}\n             STATUS    : RUNNING\n"
        ),
    )?;

    // Stage 0: evaluate the explicitly supplied DAG before any rewriting.
    let original = evaluate_dag(program)
        .map_err(|error| io::Error::other(format!("Input DAG evaluation failed: {error}")))?;
    write_dag(&original_path, &original.dag)?;
    append_result(
        result_file,
        &format!(
            "Original  : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n",
            original.stats.md, original.stats.mc, original.stats.fhe_cost,
        ),
    )?;

    // Construct and saturate exactly once. All three extractors below share
    // this same equivalent-program search space.
    let saturated = saturate_dag(program)
        .map_err(|error| io::Error::other(format!("E-graph saturation failed: {error}")))?;
    let eclass_count = saturated.egraph.number_of_classes();
    let enode_count = saturated.egraph.total_size();
    append_result(
        result_file,
        &format!("E-classes : {eclass_count}\n             E-nodes   : {enode_count}\n"),
    )?;

    // Stage 2: MD-minimal extraction.
    let md_result = extract_md(&saturated);
    write_dag(&md_path, &md_result.dag)?;
    append_result(
        result_file,
        &format!(
            "MD-opt    : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n",
            md_result.dag_stats.md, md_result.dag_stats.mc, md_result.dag_stats.fhe_cost,
        ),
    )?;

    // Stage 1: globally MC-minimal DAG extraction.
    let mc_result = extract_global_mc(&saturated);
    write_dag(&mc_path, &mc_result.dag)?;
    append_result(
        result_file,
        &format!(
            "MC-opt    : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n",
            mc_result.dag_stats.md, mc_result.dag_stats.mc, mc_result.dag_stats.fhe_cost,
        ),
    )?;

    // Stage 3: globally minimize MC under the tightest feasible MD bound.
    // This is the current MC/MD joint point: min MC subject to MD <= MD_min.
    let md_limit = md_result.dag_stats.md;
    let mcmd_result = extract_mc_under_md(&saturated, md_limit).map_err(|error| {
        io::Error::other(format!(
            "MC-under-MD extraction failed (MD <= {md_limit}): {error}"
        ))
    })?;
    write_dag(&mcmd_path, &mcmd_result.dag)?;
    append_result(
        result_file,
        &format!(
            "MCMD-opt  : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n             STATUS    : SUCCEEDED\n\n",
            mcmd_result.dag_stats.md, mcmd_result.dag_stats.mc, mcmd_result.dag_stats.fhe_cost,
        ),
    )?;

    Ok(format!(
        "============================================================\n\
         Benchmark : {name}\n\
         E-classes : {eclass_count}\n\
         E-nodes   : {enode_count}\n\
         Original  : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n\
         MD-opt    : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n\
         MC-opt    : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n\
         MCMD-opt  : MD = {:<3}  MC = {:<3}  MC × MD² = {}\n\
",
        original.stats.md,
        original.stats.mc,
        original.stats.fhe_cost,
        md_result.dag_stats.md,
        md_result.dag_stats.mc,
        md_result.dag_stats.fhe_cost,
        mc_result.dag_stats.md,
        mc_result.dag_stats.mc,
        mc_result.dag_stats.fhe_cost,
        mcmd_result.dag_stats.md,
        mcmd_result.dag_stats.mc,
        mcmd_result.dag_stats.fhe_cost,
    ))
}

fn main() -> Result<(), Box<dyn Error>> {
    let input_dir = Path::new(INPUT_DIR);
    let benchmark_paths = benchmark_files(input_dir)?;

    fs::create_dir_all(OUTPUT_DIR)?;

    // Start a fresh batch result file. From this point onward every completed
    // stage is appended and synced immediately.
    let mut result_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(RESULT_FILE)?;

    append_result(
        &mut result_file,
        &format!(
            "Batch optimization results\nInput directory : {}\nOutput directory: {}\nBenchmarks      : {}\n\n",
            input_dir.display(),
            Path::new(OUTPUT_DIR).display(),
            benchmark_paths.len(),
        ),
    )?;

    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for (index, path) in benchmark_paths.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<invalid filename>");

        println!(
            "\n[{}/{}] Processing {name}",
            index + 1,
            benchmark_paths.len()
        );

        match process_one(path, &mut result_file) {
            Ok(report) => {
                println!("{report}");
                succeeded += 1;
            }
            Err(error) => {
                let report =
                    format!("STATUS    : FAILED\n                     Error     : {error}\n\n");
                eprintln!(
                    "============================================================\n                     Benchmark : {name}\n                     {report}"
                );
                append_result(&mut result_file, &report)?;
                failed += 1;
            }
        }
    }

    append_result(
        &mut result_file,
        &format!(
            "============================================================\n             Completed: {succeeded} succeeded, {failed} failed, {} total.\n",
            benchmark_paths.len(),
        ),
    )?;

    println!("\nBatch complete: {succeeded} succeeded, {failed} failed.");
    println!("Incremental results saved to: {RESULT_FILE}");

    Ok(())
}

