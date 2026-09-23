#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Expr {
    Var(String),
    Add(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Var(String),
    Plus,
    Star,
    LParen,
    RParen,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_product()?;

        while matches!(self.peek(), Some(Token::Plus)) {
            self.next();
            let rhs = self.parse_product()?;
            expr = Expr::Add(Box::new(expr), Box::new(rhs));
        }

        Ok(expr)
    }

    fn parse_product(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_atom()?;

        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.next();
                    let rhs = self.parse_atom()?;
                    expr = Expr::Mul(Box::new(expr), Box::new(rhs));
                }

                // 隐式乘法：
                // x3x6          => x3 * x6
                // x3(x4+x5)     => x3 * (x4+x5)
                Some(Token::Var(_)) | Some(Token::LParen) => {
                    let rhs = self.parse_atom()?;
                    expr = Expr::Mul(Box::new(expr), Box::new(rhs));
                }

                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_atom(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Var(name)) => Ok(Expr::Var(name)),

            Some(Token::LParen) => {
                let expr = self.parse_expr()?;

                match self.next() {
                    Some(Token::RParen) => Ok(expr),
                    other => Err(format!("Expected ')', got {:?}", other)),
                }
            }

            other => Err(format!("Expected variable or '(', got {:?}", other)),
        }
    }
}

/// 变量格式：x1, x2, x123 ...
fn tokenize_rhs(rhs: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = rhs.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ch if ch.is_whitespace() => {
                i += 1;
            }

            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }

            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }

            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }

            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }

            'x' => {
                let start = i;
                i += 1;

                let digit_start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }

                if digit_start == i {
                    return Err(format!(
                        "Invalid variable near '{}': expected x<number>",
                        chars[start]
                    ));
                }

                let name: String = chars[start..i].iter().collect();
                tokens.push(Token::Var(name));
            }

            ch => {
                return Err(format!(
                    "Unexpected character '{}'. Only x<number>, +, *, (, ) are supported.",
                    ch
                ));
            }
        }
    }

    Ok(tokens)
}

/// 对 + 和 * 做：
/// 1. 递归规范化
/// 2. 结合律展平
/// 3. 交换律排序
/// 4. 平衡重建二叉树
fn normalize(expr: Expr) -> Expr {
    match expr {
        Expr::Var(_) => expr,

        Expr::Add(left, right) => {
            let mut terms = Vec::new();
            collect_add(normalize(*left), &mut terms);
            collect_add(normalize(*right), &mut terms);

            terms.sort_by_key(expr_sort_key);
            build_balanced_add(terms)
        }

        Expr::Mul(left, right) => {
            let mut factors = Vec::new();
            collect_mul(normalize(*left), &mut factors);
            collect_mul(normalize(*right), &mut factors);

            terms_sort_by_variable_then_structure(&mut factors);
            build_balanced_mul(factors)
        }
    }
}

fn collect_add(expr: Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::Add(left, right) => {
            collect_add(*left, out);
            collect_add(*right, out);
        }
        other => out.push(other),
    }
}

fn collect_mul(expr: Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::Mul(left, right) => {
            collect_mul(*left, out);
            collect_mul(*right, out);
        }
        other => out.push(other),
    }
}

/// 让 x2 排在 x10 前面，而不是字符串排序时的 x10 < x2。
fn terms_sort_by_variable_then_structure(exprs: &mut [Expr]) {
    exprs.sort_by_key(expr_sort_key);
}

fn expr_sort_key(expr: &Expr) -> String {
    match expr {
        Expr::Var(name) => {
            let number = name
                .strip_prefix('x')
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(usize::MAX);

            format!("0:{number:010}")
        }

        Expr::Add(_, _) => format!("1:{}", to_prefix(expr)),
        Expr::Mul(_, _) => format!("2:{}", to_prefix(expr)),
    }
}

fn build_balanced_add(mut terms: Vec<Expr>) -> Expr {
    assert!(!terms.is_empty());

    if terms.len() == 1 {
        return terms.pop().unwrap();
    }

    let mid = terms.len() / 2;
    let right = terms.split_off(mid);

    Expr::Add(
        Box::new(build_balanced_add(terms)),
        Box::new(build_balanced_add(right)),
    )
}

fn build_balanced_mul(mut factors: Vec<Expr>) -> Expr {
    assert!(!factors.is_empty());

    if factors.len() == 1 {
        return factors.pop().unwrap();
    }

    let mid = factors.len() / 2;
    let right = factors.split_off(mid);

    Expr::Mul(
        Box::new(build_balanced_mul(factors)),
        Box::new(build_balanced_mul(right)),
    )
}

fn to_prefix(expr: &Expr) -> String {
    match expr {
        Expr::Var(name) => name.clone(),

        Expr::Add(left, right) => {
            format!("(+ {} {})", to_prefix(left), to_prefix(right))
        }

        Expr::Mul(left, right) => {
            format!("(* {} {})", to_prefix(left), to_prefix(right))
        }
    }
}

/// 输入：
/// f61624 = x3x6+x3x5+x2x6
///
/// 返回：
/// ("f61624", "(+ (* x3 x6) ...)")
pub fn arithmetic_to_prefix(input: &str) -> Result<(String, String), String> {
    let (lhs, rhs) = input
        .split_once('=')
        .ok_or_else(|| "Input must contain '='.".to_string())?;

    let output_name = lhs.trim().to_string();
    if output_name.is_empty() {
        return Err("Missing output name before '='.".to_string());
    }

    let tokens = tokenize_rhs(rhs.trim())?;
    if tokens.is_empty() {
        return Err("Expression after '=' is empty.".to_string());
    }

    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;

    if let Some(token) = parser.peek() {
        return Err(format!("Unexpected remaining token: {:?}", token));
    }

    let normalized = normalize(expr);
    Ok((output_name, to_prefix(&normalized)))
}

