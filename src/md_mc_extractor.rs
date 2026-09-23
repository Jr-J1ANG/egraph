//! Global DAG extraction with a multiplication-depth upper bound.
//!
//! For a saturated e-graph and a user-supplied `md_limit = D`, this module
//! solves the following mixed-integer program:
//!
//!     minimise     MC
//!     subject to   extracted multiplication depth <= D
//!
//! The formulation is global: every used e-class selects exactly one e-node,
//! so a selected e-class is shared by all of its parents. It is therefore a
//! DAG-MC objective, not Egg's ordinary tree-extraction objective.
//!
//! This implementation additionally:
//! - models only e-classes reachable from the output root;
//! - removes candidates that cannot appear in any acyclic DAG with MD <= D;
//! - forces every used non-root e-class to be referenced by a selected parent,
//!   eliminating disconnected zero-cost components;
//! - treats depth/rank as continuous witness variables, while selection
//!   variables remain binary;
//! - returns an error rather than claiming exact optimality after a time limit.

use crate::egraph_builder::SaturatedEGraph;
use crate::lang::FheLang;

use egg::{EGraph, Id, Language, RecExpr};
use good_lp::highs;
use good_lp::{
    Expression, ProblemVariables, Solution, SolutionStatus, SolverModel, Variable, WithTimeLimit,
    variable,
};

use std::collections::{HashMap, HashSet};

/// Maximum wall-clock time for one Stage-3 MD-constrained MIP solve.
///
/// Exact mode: if the solver reaches this limit without proving optimality,
/// `solve` returns `Err` instead of returning an uncertified incumbent.
const MD_MC_EXTRACTION_TIMEOUT_SECS: f64 = 1000.0;

/// A global MIP extractor for `min MC subject to MD <= D`.
///
/// The extractor borrows a saturated e-graph, so it can be run repeatedly for
/// different depth bounds without rebuilding or re-saturating that e-graph.
pub struct MdMcExtractor<'a> {
    egraph: &'a EGraph<FheLang, ()>,
    root: Id,
}

#[derive(Clone)]
struct NodeRecord {
    /// Compact index of the e-class that owns this e-node.
    owner: usize,

    /// The selected e-node, still expressed in e-class IDs until rebuilding.
    node: FheLang,

    /// Canonical compact child e-class indices, in the same order as
    /// `node.children()`.
    children: Vec<usize>,

    /// True exactly for multiplication e-nodes.
    is_mul: bool,

    /// A safe lower bound on the multiplication depth of this implementation.
    ///
    /// It is used only to prune candidates and strengthen the relaxation; the
    /// selected DAG is still checked by the exact MD constraints below.
    min_depth: usize,

    /// Binary MIP variable: this e-node is selected for its owner e-class.
    selected: Variable,
}

impl<'a> MdMcExtractor<'a> {
    /// Construct an extractor from an e-graph and its virtual `outputs(...)`
    /// root e-class.
    pub fn new(egraph: &'a EGraph<FheLang, ()>, root: Id) -> Self {
        Self {
            egraph,
            root: egraph.find(root),
        }
    }

    /// Convenience constructor for the project's reusable saturation result.
    pub fn from_saturated(saturated: &'a SaturatedEGraph) -> Self {
        Self::new(&saturated.egraph, saturated.root)
    }

    /// Solve `min MC` subject to the extracted DAG satisfying `MD <= md_limit`.
    ///
    /// Returns `(best_expression, optimal_mc)`. This function returns `Err` if
    /// the requested bound is infeasible or if the solver times out before
    /// certifying the exact optimum.
    pub fn solve(&self, md_limit: usize) -> Result<(RecExpr<FheLang>, usize), String> {
        // A rooted extraction can only use e-classes reachable from the output
        // root through some candidate e-node. Modeling all other e-classes only
        // introduces irrelevant binary variables and symmetries.
        let class_ids = collect_root_reachable_classes(self.egraph, self.root);
        if class_ids.is_empty() {
            return Err("Cannot extract from an empty e-graph".to_string());
        }

        let mut class_index = HashMap::with_capacity(class_ids.len());
        for (index, id) in class_ids.iter().copied().enumerate() {
            class_index.insert(id, index);
        }

        let root = self.egraph.find(self.root);
        let root_index = *class_index.get(&root).ok_or_else(|| {
            "The requested extraction root is not in the root-reachable e-class set".to_string()
        })?;

        let class_count = class_ids.len();

        // Least fixed-point lower bounds on multiplication depth. They are exact
        // lower bounds even if the saturated e-graph contains cycles: a cycle
        // receives no finite value unless some acyclic alternative bottoms out at
        // leaves.
        let min_depth_lb = compute_min_depth_lower_bounds(self.egraph, &class_ids, &class_index);

        match min_depth_lb[root_index] {
            Some(lb) if lb <= md_limit => {}
            Some(lb) => {
                return Err(format!(
                    "MD bound {md_limit} is infeasible: the output root has lower-bound MD {lb}"
                ));
            }
            None => {
                return Err(
                    "The output root has no finite acyclic derivation in the saturated e-graph"
                        .to_string(),
                );
            }
        }

        let mut variables = ProblemVariables::new();

        // u[c] = 1 iff e-class c occurs in the extracted DAG.
        let used: Vec<Variable> = (0..class_count)
            .map(|_| variables.add(variable().binary()))
            .collect();

        // h[c] is a depth witness and rank[c] is a topological-order witness.
        //
        // They do not encode structural choices: those are the `selected`
        // binaries below. Continuous witnesses are sufficient:
        // - h(child) + {0,1} <= h(parent) and h <= D prove MD <= D;
        // - rank(child) + 1 <= rank(parent) prove acyclicity.
        let depth: Vec<Variable> = (0..class_count)
            .map(|_| variables.add(variable().min(0.0).max(md_limit as f64)))
            .collect();

        let rank: Vec<Variable> = (0..class_count)
            .map(|_| variables.add(variable().min(0.0).max(class_count as f64)))
            .collect();

        let mut records = Vec::<NodeRecord>::new();
        let mut nodes_by_class = vec![Vec::<usize>::new(); class_count];

        // Create x[n] only for candidates that can possibly participate in an
        // acyclic extraction with MD <= md_limit.
        for (owner, class_id) in class_ids.iter().copied().enumerate() {
            'node: for node in &self.egraph[class_id].nodes {
                let mut children = Vec::with_capacity(node.children().len());
                let mut child_depth_lb = 0usize;

                for child in node.children() {
                    let canonical_child = self.egraph.find(*child);
                    let child_index = *class_index.get(&canonical_child).ok_or_else(|| {
                        format!(
                            "Canonical child e-class {:?} is absent from the root-reachable model",
                            canonical_child
                        )
                    })?;

                    // A direct owner -> owner dependency can never be selected in
                    // an acyclic extracted DAG. Remove it before modeling instead
                    // of making the solver discover this through rank branching.
                    if child_index == owner {
                        continue 'node;
                    }

                    let Some(lb) = min_depth_lb[child_index] else {
                        // This child has no finite acyclic derivation, so this
                        // candidate can never occur in a valid extracted DAG.
                        continue 'node;
                    };

                    child_depth_lb = child_depth_lb.max(lb);
                    children.push(child_index);
                }

                let is_mul = matches!(node, FheLang::Mul(_));
                let node_min_depth = child_depth_lb + if is_mul { 1 } else { 0 };

                // Even with every child at its shallowest possible realization,
                // this implementation already exceeds the supplied MD bound.
                if node_min_depth > md_limit {
                    continue;
                }

                let record_index = records.len();
                records.push(NodeRecord {
                    owner,
                    node: node.clone(),
                    children,
                    is_mul,
                    min_depth: node_min_depth,
                    selected: variables.add(variable().binary()),
                });
                nodes_by_class[owner].push(record_index);
            }
        }

        if nodes_by_class[root_index].is_empty() {
            return Err(format!(
                "MD bound {md_limit} is infeasible: no root implementation remains after safe pruning"
            ));
        }

        // For every e-class c, record the candidate parent e-nodes that reference
        // c. A record is inserted once even for an e-node such as Mul(c, c).
        let mut parent_records = vec![Vec::<usize>::new(); class_count];
        for (record_index, record) in records.iter().enumerate() {
            let mut unique_children = HashSet::new();
            for &child in &record.children {
                if unique_children.insert(child) {
                    parent_records[child].push(record_index);
                }
            }
        }

        // The MC objective counts selected multiplication e-nodes exactly once.
        // A shared e-class has one `used` variable and selects one implementation
        // regardless of how many parents reference it.
        let mut objective = Expression::from(0.0);
        for record in &records {
            if record.is_mul {
                objective += record.selected;
            }
        }

        let mut model = variables.minimise(objective).using(highs);

        // The virtual Outputs root must occur in the extracted program.
        model = model.with((used[root_index] - 1.0).eq(0.0));

        // Every used e-class chooses exactly one implementation; an unused class
        // chooses none:
        //
        //     sum_{n in N(c)} x[n] = u[c].
        for (class, record_indices) in nodes_by_class.iter().enumerate() {
            let mut selected_sum = Expression::from(0.0);
            for &record_index in record_indices {
                selected_sum += records[record_index].selected;
            }
            model = model.with((selected_sum - used[class]).eq(0.0));
        }

        // Link auxiliary witness variables to used[c]. This is valid for every
        // integer extraction and tightens the LP relaxation:
        //
        //     lb[c] * u[c] <= depth[c] <= D * u[c]
        //     0 <= rank[c] <= C * u[c].
        for class in 0..class_count {
            model = model.with((depth[class] - (md_limit as f64) * used[class]).leq(0.0));

            if let Some(lb) = min_depth_lb[class] {
                model = model.with((depth[class] - (lb as f64) * used[class]).geq(0.0));
            }

            model = model.with((rank[class] - (class_count as f64) * used[class]).leq(0.0));
        }

        // A selected parent implementation requires every child e-class:
        //
        //     x[n] <= u[child].
        //
        // Conversely, every used non-root e-class must be referenced by at least
        // one selected parent:
        //
        //     u[c] <= sum_{n: c in children(n)} x[n].
        //
        // Together with the root constraint and acyclicity, these conditions make
        // the selected subgraph exactly root-reachable, eliminating disconnected
        // zero-MC components.
        for record in &records {
            for &child in &record.children {
                model = model.with((record.selected - used[child]).leq(0.0));
            }
        }

        for class in 0..class_count {
            if class == root_index {
                continue;
            }

            let mut selected_parent_sum = Expression::from(0.0);
            for &record_index in &parent_records[class] {
                selected_parent_sum += records[record_index].selected;
            }
            model = model.with((used[class] - selected_parent_sum).leq(0.0));
        }

        // Conditional acyclicity and depth constraints.
        //
        // Rank:
        //     rank(child) + 1 <= rank(owner)  when x[n] = 1.
        //
        // Depth:
        //     depth(child) + increment <= depth(owner)  when x[n] = 1,
        // where increment is 1 for Mul and 0 otherwise.
        //
        // The big-M values are chosen only as large as necessary from the
        // declared witness-variable bounds.
        let rank_big_m = class_count as f64 + 1.0;
        for record in &records {
            let increment = if record.is_mul { 1.0 } else { 0.0 };

            // Valid strengthening: choosing this implementation forces its owner
            // to have at least the local lower-bound depth of the implementation.
            if record.min_depth != 0 {
                model = model.with(
                    (depth[record.owner] - (record.min_depth as f64) * record.selected).geq(0.0),
                );
            }

            for &child in &record.children {
                // rank(child) - rank(owner) + R*x[n] <= R - 1.
                let acyclic = rank[child] - rank[record.owner] + rank_big_m * record.selected;
                model = model.with(acyclic.leq(rank_big_m - 1.0));

                // depth(child) - depth(owner) + M*x[n] <= M - increment.
                //
                // For Add/Neg/Outputs, M=D is sufficient. For Mul, M=D+1 is
                // necessary and sufficient because increment=1.
                let depth_big_m = md_limit as f64 + increment;
                let md_constraint =
                    depth[child] - depth[record.owner] + depth_big_m * record.selected;
                model = model.with(md_constraint.leq(depth_big_m - increment));
            }
        }

        let solution = model
            .with_time_limit(MD_MC_EXTRACTION_TIMEOUT_SECS)
            .solve()
            .map_err(|error| {
                format!(
                    "Stage-3 MIP failed for MD <= {md_limit} \
                     within {:.1}s (HiGHS/MIP: {error})",
                    MD_MC_EXTRACTION_TIMEOUT_SECS
                )
            })?;

        // Exact extractor: do not silently treat a time-limited incumbent as an
        // optimal MC value.
        match solution.status() {
            SolutionStatus::Optimal => {}
            status => {
                return Err(format!(
                    "Stage-3 MIP ended with status {status:?} for MD <= {md_limit}; \
                     no exact MC optimum was certified within {:.1}s",
                    MD_MC_EXTRACTION_TIMEOUT_SECS
                ));
            }
        }

        // Decode the selected e-node of each used e-class. With binary variables
        // there must be exactly one value above 0.5 for every used class.
        let mut selected_node = vec![None; class_count];
        for (record_index, record) in records.iter().enumerate() {
            if solution.value(record.selected) > 0.5 {
                if selected_node[record.owner].replace(record_index).is_some() {
                    return Err(format!(
                        "Solver selected more than one implementation for e-class {}",
                        record.owner
                    ));
                }
            }
        }

        if selected_node[root_index].is_none() {
            return Err("Solver did not select an implementation for the root e-class".to_string());
        }

        let mut best = RecExpr::<FheLang>::default();
        let mut rebuilt = vec![None; class_count];
        let rebuilt_root = rebuild_recexpr(
            root_index,
            &selected_node,
            &records,
            &mut best,
            &mut rebuilt,
        )?;

        // The virtual root must be the final RecExpr node. This follows from
        // post-order rebuilding; retaining the check makes model bugs explicit.
        if rebuilt_root != Id::from(best.as_ref().len() - 1) {
            return Err("Rebuilt expression root is not the final RecExpr node".to_string());
        }

        let optimal_mc = records
            .iter()
            .filter(|record| record.is_mul && solution.value(record.selected) > 0.5)
            .count();

        Ok((best, optimal_mc))
    }
}

/// Returns the canonical e-classes that can occur in an extraction rooted at
/// `root`, following all candidate e-node child links.
///
/// E-classes outside this set cannot affect any rooted output program and are
/// therefore omitted from the MIP without changing the feasible extraction set.
fn collect_root_reachable_classes(egraph: &EGraph<FheLang, ()>, root: Id) -> Vec<Id> {
    let mut reachable = HashSet::new();
    let mut todo = vec![egraph.find(root)];

    while let Some(id) = todo.pop() {
        let id = egraph.find(id);
        if !reachable.insert(id) {
            continue;
        }

        for node in &egraph[id].nodes {
            for child in node.children() {
                todo.push(egraph.find(*child));
            }
        }
    }

    // Keep Egg's class iteration order for stable compact indices and reproducible
    // MIP models.
    egraph
        .classes()
        .filter_map(|class| reachable.contains(&class.id).then_some(class.id))
        .collect()
}

/// Computes a least fixed-point lower bound on the minimum multiplication depth
/// obtainable for every modelled e-class.
///
/// `None` means that no finite acyclic derivation from leaves exists in the
/// current root-reachable candidate graph. This is a safe lower-bound analysis:
/// it may retain candidates that later fail the global sharing/acyclicity model,
/// but it never removes a candidate that could satisfy MD <= D.
fn compute_min_depth_lower_bounds(
    egraph: &EGraph<FheLang, ()>,
    class_ids: &[Id],
    class_index: &HashMap<Id, usize>,
) -> Vec<Option<usize>> {
    let mut lower = vec![None; class_ids.len()];
    let mut changed = true;

    while changed {
        changed = false;

        for (owner, class_id) in class_ids.iter().copied().enumerate() {
            let mut best = lower[owner];

            for node in &egraph[class_id].nodes {
                let mut all_children_known = true;
                let mut max_child_depth = 0usize;

                for child in node.children() {
                    let canonical_child = egraph.find(*child);
                    let Some(&child_index) = class_index.get(&canonical_child) else {
                        all_children_known = false;
                        break;
                    };

                    let Some(child_lower) = lower[child_index] else {
                        all_children_known = false;
                        break;
                    };

                    max_child_depth = max_child_depth.max(child_lower);
                }

                if !all_children_known {
                    continue;
                }

                let candidate = max_child_depth
                    + if matches!(node, FheLang::Mul(_)) {
                        1
                    } else {
                        0
                    };

                if best.map_or(true, |current| candidate < current) {
                    best = Some(candidate);
                }
            }

            if best != lower[owner] {
                lower[owner] = best;
                changed = true;
            }
        }
    }

    lower
}

/// Rebuilds a post-order `RecExpr` from the chosen e-node implementation of
/// every root-reachable e-class. Rank constraints in the MIP guarantee that this
/// recursion sees a DAG rather than an e-graph cycle.
fn rebuild_recexpr(
    class: usize,
    selected_node: &[Option<usize>],
    records: &[NodeRecord],
    expression: &mut RecExpr<FheLang>,
    memo: &mut [Option<Id>],
) -> Result<Id, String> {
    if let Some(id) = memo[class] {
        return Ok(id);
    }

    let record_index = selected_node[class].ok_or_else(|| {
        format!("Selected parent requires e-class {class}, but no implementation was selected")
    })?;
    let record = &records[record_index];

    let child_ids = record
        .children
        .iter()
        .map(|&child| rebuild_recexpr(child, selected_node, records, expression, memo))
        .collect::<Result<Vec<_>, _>>()?;

    let mut node = record.node.clone();
    let slots = node.children_mut();
    if slots.len() != child_ids.len() {
        return Err("Internal error: e-node child count changed while rebuilding".to_string());
    }

    for (slot, child_id) in slots.iter_mut().zip(child_ids) {
        *slot = child_id;
    }

    let id = expression.add(node);
    memo[class] = Some(id);
    Ok(id)
}

