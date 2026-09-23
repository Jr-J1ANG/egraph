use crate::lang::FheLang;
use egg::{Analysis, CostFunction, EGraph, Id, LpCostFunction};

/// Tree-extraction cost for minimizing multiplicative depth.
///
/// Field order matters because `Ord` is derived lexicographically:
/// 1. minimize MD;
/// 2. among equal-MD candidates, minimize tree MC;
/// 3. then prefer smaller syntax trees.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MdCost {
    pub md: usize,
    pub mc: usize,
    pub ast_size: usize,
}

impl MdCost {
    fn new(md: usize, mc: usize, ast_size: usize) -> Self {
        Self { md, mc, ast_size }
    }

    fn leaf() -> Self {
        Self::new(0, 0, 1)
    }
}

/// Tree-based extractor cost:
/// minimize MD first, then tree MC, then AST size.
///
/// MD is valid for ordinary Egg extraction because:
/// - Add: max(child MD)
/// - Mul: 1 + max(child MD)
/// - Outputs: max(output MD)
///
/// Unlike MC, MD is unaffected by DAG sharing.
pub struct MinMdTreeCost;

impl CostFunction<FheLang> for MinMdTreeCost {
    type Cost = MdCost;

    fn cost<C>(&mut self, enode: &FheLang, mut child_cost: C) -> Self::Cost
    where
        C: FnMut(Id) -> Self::Cost,
    {
        match enode {
            FheLang::Num(_) | FheLang::Symbol(_) => MdCost::leaf(),

            FheLang::Add([a, b]) => {
                let ca = child_cost(*a);
                let cb = child_cost(*b);

                MdCost::new(
                    ca.md.max(cb.md),
                    ca.mc + cb.mc,
                    1 + ca.ast_size + cb.ast_size,
                )
            }

            FheLang::Mul([a, b]) => {
                let ca = child_cost(*a);
                let cb = child_cost(*b);

                MdCost::new(
                    ca.md.max(cb.md) + 1,
                    ca.mc + cb.mc + 1,
                    1 + ca.ast_size + cb.ast_size,
                )
            }

            FheLang::Neg(a) => {
                let c = child_cost(*a);
                MdCost::new(c.md, c.mc, 1 + c.ast_size)
            }

            FheLang::Outputs(children) => {
                let mut md = 0;
                let mut mc = 0;
                let mut ast_size = 0;

                for child in children {
                    let cost = child_cost(*child);
                    md = md.max(cost.md);
                    mc += cost.mc;
                    ast_size += cost.ast_size;
                }

                MdCost::new(md, mc, ast_size)
            }
        }
    }
}

// Cost function for Egg's ILP-based global DAG extractor.
//
// Objective:
//     minimize the number of unique multiplication nodes.
//
// Every selected `Mul` e-node contributes 1.
// Additions, negations, constants, variables, and the virtual Outputs root
// contribute 0.
//
// Because LpExtractor selects one implementation per active e-class globally,
// shared e-classes are charged only once.

#[derive(Clone, Debug, Default)]
pub struct GlobalMcCost;

const MC_WEIGHT: f64 = 1.0;

impl<N> LpCostFunction<FheLang, N> for GlobalMcCost
where
    N: Analysis<FheLang>,
{
    fn node_cost(&mut self, _egraph: &EGraph<FheLang, N>, _eclass: Id, enode: &FheLang) -> f64 {
        match enode {
            FheLang::Mul(_) => MC_WEIGHT,

            FheLang::Add(_)
            | FheLang::Neg(_)
            | FheLang::Num(_)
            | FheLang::Symbol(_)
            | FheLang::Outputs(_) => 0.0,
        }
    }
}

