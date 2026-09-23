use egg::{Id, RecExpr};

use std::cmp::max;
use std::collections::{HashMap, HashSet};

use crate::lang::FheLang;

pub type DagId = usize;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DagKey {
    Num(i64),
    Symbol(String),
    Add(DagId, DagId),
    Mul(DagId, DagId),
    Neg(DagId),
}

// Arithmetic DAG with one or more roots.
//
// The virtual `outputs(...)` e-node is deliberately not materialized as a
// `DagKey`: it describes the result tuple, not an arithmetic operation.
pub struct Dag {
    nodes: Vec<DagKey>,
    roots: Vec<DagId>,
}

#[derive(Debug)]
pub struct DagStats {
    pub mc: usize,
    pub md: usize,
    pub fhe_cost: usize,
    pub total_nodes: usize,
}

impl Dag {
    pub(crate) fn from_parts(nodes: Vec<DagKey>, roots: Vec<DagId>) -> Self {
        assert!(
            !roots.is_empty(),
            "A DAG must have at least one output root"
        );

        Self { nodes, roots }
    }

    // Build a shared arithmetic DAG from an extracted expression.
    //
    // A top-level `outputs(o1, ..., on)` becomes multiple DAG roots. For
    // backward compatibility, an ordinary arithmetic root is treated as one
    // output.
    pub fn from_recexpr(expr: &RecExpr<FheLang>) -> Self {
        assert!(!expr.as_ref().is_empty(), "Expression must not be empty");

        // Source-tree node -> DAG node.
        let mut source_memo: HashMap<Id, DagId> = HashMap::new();

        // Structural key -> unique DAG node.
        let mut hashcons_table: HashMap<DagKey, DagId> = HashMap::new();

        let mut nodes: Vec<DagKey> = Vec::new();
        let root_expr_id = Id::from(expr.as_ref().len() - 1);

        let roots = match &expr.as_ref()[usize::from(root_expr_id)] {
            FheLang::Outputs(children) => {
                assert!(
                    !children.is_empty(),
                    "The virtual outputs node must have at least one child"
                );
                children
                    .iter()
                    .map(|child| {
                        intern_expr(
                            expr,
                            *child,
                            &mut source_memo,
                            &mut hashcons_table,
                            &mut nodes,
                        )
                    })
                    .collect()
            }
            _ => vec![intern_expr(
                expr,
                root_expr_id,
                &mut source_memo,
                &mut hashcons_table,
                &mut nodes,
            )],
        };

        Self { nodes, roots }
    }

    pub fn stats(&self) -> DagStats {
        let mut reachable: HashSet<DagId> = HashSet::new();
        for &root in &self.roots {
            collect_reachable(&self.nodes, root, &mut reachable);
        }

        let mut depth_memo: HashMap<DagId, usize> = HashMap::new();
        let md = self
            .roots
            .iter()
            .map(|&root| dag_md(&self.nodes, root, &mut depth_memo))
            .max()
            .unwrap_or(0);

        let mc = reachable
            .iter()
            .filter(|&&id| matches!(&self.nodes[id], DagKey::Mul(_, _)))
            .count();

        DagStats {
            mc,
            md,
            fhe_cost: mc.saturating_mul(md).saturating_mul(md),
            total_nodes: reachable.len(),
        }
    }

    pub fn nodes(&self) -> &[DagKey] {
        &self.nodes
    }

    pub fn roots(&self) -> &[DagId] {
        &self.roots
    }
}

fn intern_expr(
    expr: &RecExpr<FheLang>,
    source_id: Id,
    source_memo: &mut HashMap<Id, DagId>,
    hashcons_table: &mut HashMap<DagKey, DagId>,
    nodes: &mut Vec<DagKey>,
) -> DagId {
    // The same node in the input expression has already been visited.
    if let Some(&dag_id) = source_memo.get(&source_id) {
        return dag_id;
    }

    let enode = &expr.as_ref()[usize::from(source_id)];

    let key = match enode {
        FheLang::Num(n) => DagKey::Num(*n),

        FheLang::Symbol(symbol) => DagKey::Symbol(symbol.to_string()),

        FheLang::Add([a, b]) => {
            let left = intern_expr(expr, *a, source_memo, hashcons_table, nodes);
            let right = intern_expr(expr, *b, source_memo, hashcons_table, nodes);
            DagKey::Add(left, right)
        }

        FheLang::Mul([a, b]) => {
            let left = intern_expr(expr, *a, source_memo, hashcons_table, nodes);
            let right = intern_expr(expr, *b, source_memo, hashcons_table, nodes);
            DagKey::Mul(left, right)
        }

        FheLang::Neg(a) => {
            let child = intern_expr(expr, *a, source_memo, hashcons_table, nodes);
            DagKey::Neg(child)
        }

        // Only the top-level virtual node is allowed. Nested outputs would
        // have tuple semantics, which this arithmetic DAG intentionally lacks.
        FheLang::Outputs(_) => {
            panic!("`outputs` may only appear as the top-level extraction root")
        }
    };

    // Structural hash-consing:
    // if exactly the same structure exists, reuse its DAG node.
    let dag_id = if let Some(&existing_id) = hashcons_table.get(&key) {
        existing_id
    } else {
        let new_id = nodes.len();
        nodes.push(key.clone());
        hashcons_table.insert(key, new_id);
        new_id
    };

    source_memo.insert(source_id, dag_id);
    dag_id
}

fn collect_reachable(nodes: &[DagKey], id: DagId, visited: &mut HashSet<DagId>) {
    if !visited.insert(id) {
        return;
    }

    match &nodes[id] {
        DagKey::Num(_) | DagKey::Symbol(_) => {}

        DagKey::Add(a, b) | DagKey::Mul(a, b) => {
            collect_reachable(nodes, *a, visited);
            collect_reachable(nodes, *b, visited);
        }

        DagKey::Neg(a) => {
            collect_reachable(nodes, *a, visited);
        }
    }
}

fn dag_md(nodes: &[DagKey], id: DagId, memo: &mut HashMap<DagId, usize>) -> usize {
    if let Some(&cached) = memo.get(&id) {
        return cached;
    }

    let depth = match &nodes[id] {
        DagKey::Num(_) | DagKey::Symbol(_) => 0,
        DagKey::Add(a, b) => max(dag_md(nodes, *a, memo), dag_md(nodes, *b, memo)),
        DagKey::Mul(a, b) => max(dag_md(nodes, *a, memo), dag_md(nodes, *b, memo)) + 1,
        DagKey::Neg(a) => dag_md(nodes, *a, memo),
    };

    memo.insert(id, depth);
    depth
}

