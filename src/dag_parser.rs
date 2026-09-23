use egg::{EGraph, Id, Symbol};

use std::collections::HashMap;

use crate::dag::{Dag, DagKey};
use crate::lang::FheLang;

pub type NodeId = usize;

// A simple arithmetic DAG IR with one or more outputs.
//
// Input example:
//
// n0 = a
// n1 = b
// n2 = n0 * n1
// n3 = n2 + a
// outputs = n2, n3
//
// `output = nX` remains accepted as a backward-compatible one-output alias.
// Nodes must be declared in topological order: a node may refer only to an
// earlier `nX` definition.
#[derive(Clone, Debug, Default)]
pub struct ProgramDag {
    nodes: Vec<ProgramNode>,
    outputs: Vec<NodeId>,
}

#[derive(Clone, Debug)]
enum ProgramNode {
    Constant(i64),
    Input(String),
    Add(NodeId, NodeId),
    Mul(NodeId, NodeId),
    Neg(NodeId),
}

impl ProgramDag {
    /// Convert this parsed textual program directly into the internal DAG.
    ///
    /// Unlike `to_egraph`, this does not run rewriting or extraction.
    /// Every input program node becomes exactly one DAG node, so the input's
    /// explicit sharing structure is preserved.
    pub fn to_dag(&self) -> Result<Dag, String> {
        if self.outputs.is_empty() {
            return Err("ProgramDag has no output nodes".to_string());
        }

        let nodes = self
            .nodes
            .iter()
            .map(|node| match node {
                ProgramNode::Constant(value) => DagKey::Num(*value),

                ProgramNode::Input(name) => DagKey::Symbol(name.clone()),

                ProgramNode::Add(left, right) => DagKey::Add(*left, *right),

                ProgramNode::Mul(left, right) => DagKey::Mul(*left, *right),

                ProgramNode::Neg(child) => DagKey::Neg(*child),
            })
            .collect();

        Ok(Dag::from_parts(nodes, self.outputs.clone()))
    }

    /// Parse a textual DAG program.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut dag = ProgramDag::default();
        let mut outputs_declared = false;

        // Textual name such as "n3" -> internal node ID.
        let mut names: HashMap<String, NodeId> = HashMap::new();

        for (line_index, raw_line) in text.lines().enumerate() {
            let line_no = line_index + 1;

            // Support both "#" and "//" comments.
            let line = raw_line
                .split('#')
                .next()
                .unwrap_or("")
                .split("//")
                .next()
                .unwrap_or("")
                .trim();

            if line.is_empty() {
                continue;
            }

            let (lhs, rhs) = line
                .split_once('=')
                .ok_or_else(|| format!("Line {line_no}: expected `=`"))?;

            let lhs = lhs.trim();
            let rhs = rhs.trim();

            if lhs.is_empty() || rhs.is_empty() {
                return Err(format!("Line {line_no}: invalid assignment"));
            }

            // Multi-output declaration. `output = n8` is retained as a
            // backward-compatible alias for a one-element output list.
            if lhs == "output" || lhs == "outputs" {
                if outputs_declared {
                    return Err(format!(
                        "Line {line_no}: outputs are already defined; use one `outputs = ...` declaration"
                    ));
                }

                let output_names = parse_output_names(rhs, line_no)?;

                if lhs == "output" && output_names.len() != 1 {
                    return Err(format!(
                        "Line {line_no}: `output` accepts one node; use `outputs = n1, n2, ...` for multiple outputs"
                    ));
                }

                let mut outputs = Vec::with_capacity(output_names.len());
                for output_name in output_names {
                    let output_id = names.get(output_name).copied().ok_or_else(|| {
                        format!("Line {line_no}: output references undefined node `{output_name}`")
                    })?;
                    outputs.push(output_id);
                }

                dag.outputs = outputs;
                outputs_declared = true;
                continue;
            }

            if names.contains_key(lhs) {
                return Err(format!("Line {line_no}: node `{lhs}` is already defined"));
            }

            let tokens: Vec<&str> = rhs.split_whitespace().collect();

            let node = match tokens.as_slice() {
                // n0 = a
                // n0 = 17
                [value] => match value.parse::<i64>() {
                    Ok(number) => ProgramNode::Constant(number),
                    Err(_) => ProgramNode::Input((*value).to_string()),
                },

                // n1 = -n0
                ["-", child_name] => {
                    let child = lookup_node(&names, child_name, line_no)?;
                    ProgramNode::Neg(child)
                }

                // n2 = n0 + n1
                [left_name, "+", right_name] => {
                    let left = lookup_node(&names, left_name, line_no)?;
                    let right = lookup_node(&names, right_name, line_no)?;
                    ProgramNode::Add(left, right)
                }

                // n2 = n0 * n1
                [left_name, "*", right_name] => {
                    let left = lookup_node(&names, left_name, line_no)?;
                    let right = lookup_node(&names, right_name, line_no)?;
                    ProgramNode::Mul(left, right)
                }

                _ => {
                    return Err(format!("Line {line_no}: unsupported expression `{rhs}`"));
                }
            };

            let node_id = dag.nodes.len();
            dag.nodes.push(node);
            names.insert(lhs.to_string(), node_id);
        }

        if !outputs_declared {
            return Err(
                "Program has no `output = nX` or `outputs = n1, n2, ...` declaration".to_string(),
            );
        }

        Ok(dag)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    // Convert the input DAG directly into an egg EGraph.
    //
    // The shared references in ProgramDag remain shared through e-class IDs.
    // A final `outputs(...)` e-node is added as the sole extraction root. It
    // has no rewrite rule and no arithmetic cost.
    pub fn to_egraph(&self) -> Result<(EGraph<FheLang, ()>, Id), String> {
        if self.outputs.is_empty() {
            return Err("ProgramDag has no output nodes".to_string());
        }

        let mut egraph = EGraph::<FheLang, ()>::default();

        // ProgramDag node ID -> egg e-class ID.
        let mut eclass_ids: Vec<Id> = Vec::with_capacity(self.nodes.len());

        for node in &self.nodes {
            let enode = match node {
                ProgramNode::Constant(value) => FheLang::Num(*value),

                ProgramNode::Input(name) => FheLang::Symbol(Symbol::from(name.as_str())),

                ProgramNode::Add(left, right) => {
                    FheLang::Add([eclass_ids[*left], eclass_ids[*right]])
                }

                ProgramNode::Mul(left, right) => {
                    FheLang::Mul([eclass_ids[*left], eclass_ids[*right]])
                }

                ProgramNode::Neg(child) => FheLang::Neg(eclass_ids[*child]),
            };

            let eclass_id = egraph.add(enode);
            eclass_ids.push(eclass_id);
        }

        let output_children: Vec<Id> = self
            .outputs
            .iter()
            .map(|&output| eclass_ids[output])
            .collect();
        let root = egraph.add(FheLang::Outputs(output_children.into_boxed_slice()));

        egraph.rebuild();
        let root = egraph.find(root);
        Ok((egraph, root))
    }
}

fn parse_output_names<'a>(rhs: &'a str, line_no: usize) -> Result<Vec<&'a str>, String> {
    let names: Vec<&str> = rhs
        .split(',')
        .flat_map(str::split_whitespace)
        .filter(|name| !name.is_empty())
        .collect();

    if names.is_empty() {
        return Err(format!("Line {line_no}: expected at least one output node"));
    }

    Ok(names)
}

fn lookup_node(
    names: &HashMap<String, NodeId>,
    node_name: &str,
    line_no: usize,
) -> Result<NodeId, String> {
    names.get(node_name).copied().ok_or_else(|| {
        format!("Line {line_no}: node `{node_name}` must be defined before it is used")
    })
}

