use egg::{FromOp, Id, Language, Symbol};

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Arithmetic language:
/// - Add / Mul are commutative at the e-node equality level.
/// - They remain binary and non-associative structurally:
///   (a*b)*c and a*(b*c) are distinct candidates for MD optimization.
#[derive(Clone, Debug)]
pub enum FheLang {
    Num(i64),

    Add([Id; 2]),
    Mul([Id; 2]),
    Neg(Id),

    // Variadic, virtual, zero-cost program root.
    Outputs(Box<[Id]>),

    Symbol(Symbol),
}

impl FheLang {
    /// Canonical unordered view used only for equality / hashing / ordering.
    ///
    /// The stored child order is deliberately retained. Ordinary Egg patterns
    /// remain ordered, so rules.rs must include any required left/right variants.
    #[inline]
    fn unordered_pair(ids: &[Id; 2]) -> (Id, Id) {
        if ids[0] <= ids[1] {
            (ids[0], ids[1])
        } else {
            (ids[1], ids[0])
        }
    }

    #[inline]
    fn tag(&self) -> u8 {
        match self {
            Self::Num(_) => 0,
            Self::Add(_) => 1,
            Self::Mul(_) => 2,
            Self::Neg(_) => 3,
            Self::Outputs(_) => 4,
            Self::Symbol(_) => 5,
        }
    }
}

impl PartialEq for FheLang {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for FheLang {}

impl PartialOrd for FheLang {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FheLang {
    fn cmp(&self, other: &Self) -> Ordering {
        use FheLang::*;

        match (self, other) {
            (Num(a), Num(b)) => a.cmp(b),

            // Built-in commutativity.
            (Add(a), Add(b)) => Self::unordered_pair(a).cmp(&Self::unordered_pair(b)),
            (Mul(a), Mul(b)) => Self::unordered_pair(a).cmp(&Self::unordered_pair(b)),

            (Neg(a), Neg(b)) => a.cmp(b),
            (Outputs(a), Outputs(b)) => a.as_ref().cmp(b.as_ref()),
            (Symbol(a), Symbol(b)) => a.cmp(b),

            _ => self.tag().cmp(&other.tag()),
        }
    }
}

impl Hash for FheLang {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tag().hash(state);

        match self {
            Self::Num(value) => value.hash(state),

            // Built-in commutativity.
            Self::Add(children) | Self::Mul(children) => {
                let (left, right) = Self::unordered_pair(children);
                left.hash(state);
                right.hash(state);
            }

            Self::Neg(child) => child.hash(state),
            Self::Outputs(children) => children.hash(state),
            Self::Symbol(symbol) => symbol.hash(state),
        }
    }
}

impl fmt::Display for FheLang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Num(value) => write!(f, "{value}"),
            Self::Add(_) => write!(f, "+"),
            Self::Mul(_) => write!(f, "*"),
            Self::Neg(_) => write!(f, "-"),
            Self::Outputs(_) => write!(f, "outputs"),
            Self::Symbol(symbol) => write!(f, "{symbol}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FheLangDiscriminant {
    Num,
    Add,
    Mul,
    Neg,
    Outputs,
    Symbol,
}

impl Language for FheLang {
    type Discriminant = FheLangDiscriminant;

    fn discriminant(&self) -> Self::Discriminant {
        match self {
            Self::Num(_) => FheLangDiscriminant::Num,
            Self::Add(_) => FheLangDiscriminant::Add,
            Self::Mul(_) => FheLangDiscriminant::Mul,
            Self::Neg(_) => FheLangDiscriminant::Neg,
            Self::Outputs(_) => FheLangDiscriminant::Outputs,
            Self::Symbol(_) => FheLangDiscriminant::Symbol,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            // Constants and symbols are distinguished by their values.
            (Self::Num(a), Self::Num(b)) => a == b,
            (Self::Symbol(a), Self::Symbol(b)) => a == b,

            // Operators match by operator kind, not by child IDs.
            (Self::Add(_), Self::Add(_)) => true,
            (Self::Mul(_), Self::Mul(_)) => true,
            (Self::Neg(_), Self::Neg(_)) => true,

            // Outputs is variadic, so its arity is part of the operator shape.
            (Self::Outputs(a), Self::Outputs(b)) => a.len() == b.len(),

            _ => false,
        }
    }

    fn children(&self) -> &[Id] {
        match self {
            Self::Num(_) | Self::Symbol(_) => &[],
            Self::Add(children) | Self::Mul(children) => children,
            Self::Neg(child) => std::slice::from_ref(child),
            Self::Outputs(children) => children.as_ref(),
        }
    }

    fn children_mut(&mut self) -> &mut [Id] {
        match self {
            Self::Num(_) | Self::Symbol(_) => &mut [],
            Self::Add(children) | Self::Mul(children) => children,
            Self::Neg(child) => std::slice::from_mut(child),
            Self::Outputs(children) => children.as_mut(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FheLangParseError(pub String);

impl fmt::Display for FheLangParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FheLangParseError {}

impl FromOp for FheLang {
    type Error = FheLangParseError;

    fn from_op(op: &str, children: Vec<Id>) -> Result<Self, Self::Error> {
        match op {
            "+" if children.len() == 2 => Ok(Self::Add([children[0], children[1]])),
            "*" if children.len() == 2 => Ok(Self::Mul([children[0], children[1]])),
            "-" if children.len() == 1 => Ok(Self::Neg(children[0])),
            "outputs" => Ok(Self::Outputs(children.into_boxed_slice())),

            _ if children.is_empty() => {
                if let Ok(value) = op.parse::<i64>() {
                    Ok(Self::Num(value))
                } else {
                    Ok(Self::Symbol(Symbol::from(op)))
                }
            }

            _ => Err(FheLangParseError(format!(
                "Invalid FheLang node: operator `{op}` with {} children",
                children.len()
            ))),
        }
    }
}

