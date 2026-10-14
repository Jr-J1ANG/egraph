use egg::{EGraph, Id, Language};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};

use egraph_builder::{saturate_egraph_local, InputKind, SaturatedEGraph};
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

/// Find one parent e-class and the particular parent enode that references `child`.
/// Parent IDs are enode IDs; `egraph.find(parent_enode_id)` gives their current e-class.
fn find_parent_enode(
    egraph: &EGraph<lang::FheLang, ()>,
    child: Id,
    root: Id,
) -> Option<(Id, Id)> {
    let child = egraph.find(child);
    let root = egraph.find(root);

    for parent_enode_id in egraph[child].parents() {
        let parent_class = egraph.find(parent_enode_id);
        if parent_class == root {
            continue;
        }

        let Some(parent_enode) = egraph.nodes().get(usize::from(parent_enode_id)) else {
            continue;
        };
        if parent_enode
            .children()
            .iter()
            .any(|&id| egraph.find(id) == child)
        {
            return Some((parent_class, parent_enode_id));
        }
    }

    None
}

/// Collect a subtree down to `depth` edges. `enode_id` pins the exact enode
/// used by the selected parent chain; otherwise the first enode in the e-class
/// is used to continue through the neighboring branch.
///
/// `forced_child_enode` ensures that the selected parent of the seed is used
/// when descending from the selected grandparent, preserving the chosen path.
fn collect_subtree(
    egraph: &EGraph<lang::FheLang, ()>,
    class_id: Id,
    enode_id: Option<Id>,
    depth: usize,
    root: Id,
    forced_child_enode: Option<(Id, Id)>,
    scope: &mut Vec<Id>,
    seen: &mut HashSet<Id>,
    expanded_depth: &mut HashMap<Id, usize>,
) {
    let class_id = egraph.find(class_id);
    let root = egraph.find(root);
    if class_id == root {
        return;
    }

    seen.insert(class_id);
    if expanded_depth
        .get(&class_id)
        .is_some_and(|&previous_depth| previous_depth >= depth)
    {
        return;
    }
    expanded_depth.insert(class_id, depth);

    let node = if let Some(id) = enode_id {
        egraph.nodes().get(usize::from(id))
    } else {
        egraph[class_id].nodes.first()
    };
    let Some(node) = node else {
        return;
    };

    if !scope.contains(&class_id) {
        scope.push(class_id);
    }
    if depth == 0 {
        return;
    }

    for &child_id in node.children() {
        let child = egraph.find(child_id);
        if child == root {
            continue;
        }
        let child_enode_id = forced_child_enode
            .filter(|(forced_class, _)| egraph.find(*forced_class) == child)
            .map(|(_, forced_enode)| forced_enode);
        collect_subtree(
            egraph,
            child,
            child_enode_id,
            depth - 1,
            root,
            None,
            scope,
            seen,
            expanded_depth,
        );
    }
}

/// Choose one unparticipated seed, then find a parent and grandparent.
/// The local scope is the grandparent-rooted binary neighborhood through two
/// child edges (at most seven e-classes for a tree with binary enodes).
/// If the seed has no valid parent/grandparent, use the highest available node
/// on that chain as the neighborhood root.
fn choose_scope(
    egraph: &EGraph<lang::FheLang, ()>,
    unparticipated: &[Id],
    root: Id,
) -> Vec<Id> {
    let root = egraph.find(root);
    let Some(seed) = unparticipated
        .iter()
        .map(|&id| egraph.find(id))
        .find(|&id| id != root)
    else {
        return Vec::new();
    };

    let (scope_root, scope_root_enode, forced_child_enode) =
        match find_parent_enode(egraph, seed, root) {
            Some((parent, parent_enode)) => match find_parent_enode(egraph, parent, root) {
                Some((grandparent, grandparent_enode)) => (
                    grandparent,
                    Some(grandparent_enode),
                    Some((parent, parent_enode)),
                ),
                None => (parent, Some(parent_enode), None),
            },
            None => (seed, None, None),
        };

    let mut scope = Vec::new();
    let mut seen = HashSet::new();
    let mut expanded_depth = HashMap::new();
    collect_subtree(
        egraph,
        scope_root,
        scope_root_enode,
        2,
        root,
        forced_child_enode,
        &mut scope,
        &mut seen,
        &mut expanded_depth,
    );

    // The selected chain guarantees the seed is in the subtree in normal DAG
    // cases; retain it explicitly for unusual e-graph sharing/representatives.
    if seed != root && seen.insert(seed) {
        scope.push(seed);
    }
    scope.sort_unstable();
    scope
}

fn wrap_pruned_egraph(
    egraph: egg::EGraph<lang::FheLang, ()>,
    root: Id,
    node_count: usize,
    output_count: usize,
) -> SaturatedEGraph {
    let root = egraph.find(root);
    SaturatedEGraph {
        egraph,
        root,
        input: InputKind::Dag {
            node_count,
            output_count,
        },
        iterations: 0,
        stop_reason: "local saturation on current e-graph".to_string(),
    }
}

/// Count multiplication e-classes reachable from the selected root.
/// This is intended for a pruned EGraph, where each reachable e-class
/// represents the selected implementation and sharing is counted once.
fn count_pruned_mc(egraph: &EGraph<lang::FheLang, ()>, root: Id) -> usize {
    let mut stack = vec![egraph.find(root)];
    let mut seen = HashSet::new();
    let mut mc = 0usize;

    while let Some(id) = stack.pop() {
        let id = egraph.find(id);
        if !seen.insert(id) {
            continue;
        }

        let class = &egraph[id];
        let Some(node) = class.nodes.first() else {
            continue;
        };

        if matches!(node, lang::FheLang::Mul(_)) {
            mc += 1;
        }
        stack.extend(node.children().iter().copied());
    }

    mc
}

/// Build a fresh raw EGraph and initialize the first local scope from its
/// actual canonical e-class IDs. The virtual Outputs root is deliberately
/// excluded from the set of arithmetic/input classes to be processed.
fn build_initial_local_state(
    parsed: &dag_parser::ProgramDag,
) -> Result<
    (
        egraph_builder::SaturatedEGraph,
        Vec<Id>,
        Vec<Id>,
        egraph_builder::SaturatedEGraph,
    ),
    Box<dyn std::error::Error>,
> {
    let (raw_egraph, raw_root) = parsed
        .to_egraph()
        .map_err(|e| io::Error::other(format!("Failed to build raw EGraph: {e}")))?;
    let root = raw_egraph.find(raw_root);

    let mut unparticipated: Vec<Id> = raw_egraph
        .classes()
        .map(|eclass| raw_egraph.find(eclass.id))
        .filter(|&id| id != root)
        .collect();
    unparticipated.sort_unstable();
    unparticipated.dedup();

    // Keep an untouched raw graph as the accepted state to roll back to if
    // the first MC extraction is worse than the baseline.
    let raw_state = wrap_pruned_egraph(
        raw_egraph.clone(),
        root,
        parsed.node_count(),
        parsed.output_count(),
    );

    let current_scope = choose_scope(&raw_egraph, &unparticipated, root);
    let egraph = saturate_egraph_local(raw_egraph, current_scope.clone());
    let saturated = wrap_pruned_egraph(
        egraph,
        root,
        parsed.node_count(),
        parsed.output_count(),
    );

    Ok((saturated, current_scope, unparticipated, raw_state))
}

fn run_md(parsed: &dag_parser::ProgramDag) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n========== Local MD optimization ==========");
    let (mut saturated, mut current_scope, mut unparticipated, _raw_state) =
        build_initial_local_state(parsed)?;
    let node_count = parsed.node_count();
    let output_count = parsed.output_count();

    let mut round = 1usize;
    loop {
        let (pruned, _root, next_unparticipated) =
            extract_md_prune(&saturated, current_scope.clone(), unparticipated.clone());
        unparticipated = next_unparticipated;

        // Report the selected DAG's global MD/MC after every MD round, not just
        // the e-graph size. Clone the pruned graph because the next round reuses it.
        let round_egraph = saturate_egraph_local(pruned.clone(), Vec::new());
        let round_saturated = wrap_pruned_egraph(round_egraph, _root, node_count, output_count);
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
            let final_saturated = wrap_pruned_egraph(final_egraph, _root, node_count, output_count);
            let result = optimizer::extract_md(&final_saturated);
            println!(
                "[MD] final: MD={}, MC={}, nodes={}",
                result.dag_stats.md, result.dag_stats.mc, result.dag_stats.total_nodes
            );
            write_dag_text("dag_md.txt", &result.dag)?;
            break;
        }

        current_scope = choose_scope(&pruned, &unparticipated, _root);
        let next_egraph = saturate_egraph_local(pruned, current_scope.clone());
        saturated = wrap_pruned_egraph(next_egraph, _root, node_count, output_count);
        round += 1;
    }
    Ok(())
}

fn run_mc(parsed: &dag_parser::ProgramDag) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n========== Local MC optimization ==========");
    let (mut saturated, mut current_scope, mut unparticipated, mut accepted_state) =
        build_initial_local_state(parsed)?;
    let node_count = parsed.node_count();
    let output_count = parsed.output_count();

    // Baseline is the MC of the untouched raw DAG representation.
    let mut accepted_mc = count_pruned_mc(&accepted_state.egraph, accepted_state.root);
    let mut round = 1usize;

    loop {
        println!(
            "[MC] round {round}: trying scope={}, remaining_before={}, accepted_MC={}",
            current_scope.len(),
            unparticipated.len(),
            accepted_mc
        );

        let (pruned, pruned_root, next_unparticipated) =
            extract_global_mc_prune(&saturated, current_scope.clone(), unparticipated.clone());
        let candidate_mc = count_pruned_mc(&pruned, pruned_root);

        if candidate_mc > accepted_mc {
            println!(
                "[MC] round {round}: REJECTED (MC {} -> {}); rolling back and skipping this scope",
                accepted_mc, candidate_mc
            );

            // Roll back to the previous accepted pruned EGraph and its
            // unparticipated set. Remove only IDs from this attempted scope
            // that are actually present in that set; extra scope IDs are fine.
            let failed_scope: HashSet<Id> = current_scope
                .iter()
                .map(|&id| accepted_state.egraph.find(id))
                .collect();

            unparticipated.retain(|&id| {
                !failed_scope.contains(&accepted_state.egraph.find(id))
            });

            if unparticipated.is_empty() {
                println!("[MC] no untried seeds remain; keeping the last accepted graph.");
                break;
            }

            current_scope = choose_scope(
                &accepted_state.egraph,
                &unparticipated,
                accepted_state.root,
            );
            let next_egraph =
                saturate_egraph_local(accepted_state.egraph.clone(), current_scope.clone());
            saturated = wrap_pruned_egraph(
                next_egraph,
                accepted_state.root,
                node_count,
                output_count,
            );
            round += 1;
            continue;
        }

        // Accept this round, even if MC is unchanged. The pruning routine's
        // participation bookkeeping becomes the accepted history.
        accepted_mc = candidate_mc;
        unparticipated = next_unparticipated;
        println!(
            "[MC] round {round}: accepted MC={}, remaining={}, eclasses={}, enodes={}",
            accepted_mc,
            unparticipated.len(),
            pruned.number_of_classes(),
            pruned.total_size()
        );

        accepted_state = wrap_pruned_egraph(
            pruned,
            pruned_root,
            node_count,
            output_count,
        );

        if unparticipated.is_empty() {
            break;
        }

        current_scope = choose_scope(
            &accepted_state.egraph,
            &unparticipated,
            accepted_state.root,
        );
        let next_egraph =
            saturate_egraph_local(accepted_state.egraph.clone(), current_scope.clone());
        saturated = wrap_pruned_egraph(
            next_egraph,
            accepted_state.root,
            node_count,
            output_count,
        );
        round += 1;
    }

    // Final extraction is performed only on the last accepted graph, never on
    // a rejected candidate.
    let result = optimizer::extract_global_mc(&accepted_state);
    println!(
        "[MC] final accepted result: MD={}, MC={}, nodes={}",
        result.dag_stats.md, result.dag_stats.mc, result.dag_stats.total_nodes
    );
    write_dag_text("dag_mc.txt", &result.dag)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = fs::read_to_string(INPUT_DAG_PATH)?;
    let parsed = dag_parser::ProgramDag::parse(&program)
        .map_err(|e| io::Error::other(format!("Failed to parse {INPUT_DAG_PATH}: {e}")))?;
    let node_count = parsed.node_count();

    println!("Input DAG: {INPUT_DAG_PATH}");
    println!("Arithmetic/input nodes: {node_count}");
    println!("Local scope strategy: one seed + parent/grandparent-rooted depth-2 subtree (up to 7 nodes on a binary tree)");

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
    run_md(&parsed)?;
    run_mc(&parsed)?;

    println!("\nSaved optimized DAGs: dag_md.txt and dag_mc.txt");
    Ok(())
}
