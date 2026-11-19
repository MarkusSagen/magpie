//! Function plotting for the Calculator's Graphs view (Phase 7, screen 12).
//!
//! A tiny recursive-descent parser compiles an expression in one variable `x`
//! into an AST once, then evaluates it cheaply at many sample points. On top of
//! that sit sampling, numeric intersection finding (sign-change + bisection) and
//! trapezoidal area between two curves. This is deliberately separate from
//! `eval` (which evaluates *constant* expressions exactly); plotting needs a
//! fast, repeatable `f(x): f64 -> f64` and tolerates floating-point.

/// A compiled expression in the single variable `x`.
#[derive(Debug, Clone)]
pub struct Expr {
    ast: Node,
}

#[derive(Debug, Clone)]
enum Node {
    Num(f64),
    Var,
    Neg(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
    Call(Func, Box<Node>),
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, Copy)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Ln,
    Log10,
    Log2,
    Sqrt,
    Cbrt,
    Abs,
    Floor,
    Ceil,
    Round,
    Sign,
    Gamma,
}

impl Expr {
    /// Parse `src` (an expression in `x`). An optional leading `f(x) =`, `y =`,
    /// or a plain `<name>(x) =` prefix is stripped so both `sin(x)` and
    /// `f(x) = sin(x)` compile. Returns `None` on a syntax error.
    pub fn parse(src: &str) -> Option<Expr> {
        let body = strip_definition_prefix(src);
        let toks = tokenize(body)?;
        let mut p = Parser { toks, pos: 0 };
        let ast = p.parse_expr()?;
        if p.pos != p.toks.len() {
            return None;
        }
        Some(Expr { ast })
    }

    /// Evaluate at `x`. Returns `None` for undefined results (NaN/∞), so callers
    /// can break the plotted line across discontinuities.
    pub fn eval(&self, x: f64) -> Option<f64> {
        let v = eval_node(&self.ast, x);
        if v.is_finite() {
            Some(v)
        } else {
            None
        }
    }
}

/// Strip a leading `f(x) =` / `y =` / `<name> =` definition so the remainder is
/// a bare expression. Only strips a `=` that is not part of `==`/`<=`/`>=`.
fn strip_definition_prefix(src: &str) -> &str {
    if let Some(eq) = src.find('=') {
        let bytes = src.as_bytes();
        let is_cmp = eq + 1 < bytes.len() && bytes[eq + 1] == b'='
            || eq > 0 && matches!(bytes[eq - 1], b'<' | b'>' | b'!' | b'=');
        let lhs = src[..eq].trim();
        // Only treat as a definition when the LHS looks like `name` or `name(x)`.
        let looks_like_def = !lhs.is_empty()
            && lhs
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '(' | ')' | 'x' | '_' | ' '));
        if !is_cmp && looks_like_def {
            return src[eq + 1..].trim();
        }
    }
    src.trim()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let mut toks = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '+' => {
                toks.push(Tok::Plus);
                i += 1;
            }
            '-' | '−' => {
                toks.push(Tok::Minus);
                i += 1;
            }
            '*' | '·' | '×' => {
                toks.push(Tok::Star);
                i += 1;
            }
            '/' | '÷' => {
                toks.push(Tok::Slash);
                i += 1;
            }
            '^' => {
                toks.push(Tok::Caret);
                i += 1;
            }
            '(' | '[' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' | ']' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let num: String = chars[start..i].iter().collect();
                toks.push(Tok::Num(num.parse().ok()?));
            }
            _ if c.is_alphabetic() || c == '_' || c == 'π' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == 'π')
                {
                    i += 1;
                }
                toks.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            _ => return None,
        }
    }
    Some(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    // expr := term (('+'|'-') term)*
    fn parse_expr(&mut self) -> Option<Node> {
        let mut lhs = self.parse_term()?;
        while let Some(op) = match self.peek() {
            Some(Tok::Plus) => Some(Op::Add),
            Some(Tok::Minus) => Some(Op::Sub),
            _ => None,
        } {
            self.pos += 1;
            let rhs = self.parse_term()?;
            lhs = Node::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Some(lhs)
    }

    // term := unary (('*'|'/') unary)*
    fn parse_term(&mut self) -> Option<Node> {
        let mut lhs = self.parse_unary()?;
        while let Some(op) = match self.peek() {
            Some(Tok::Star) => Some(Op::Mul),
            Some(Tok::Slash) => Some(Op::Div),
            _ => None,
        } {
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = Node::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Some(lhs)
    }

    // unary := ('-'|'+') unary | power
    fn parse_unary(&mut self) -> Option<Node> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.pos += 1;
                Some(Node::Neg(Box::new(self.parse_unary()?)))
            }
            Some(Tok::Plus) => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    // power := atom ('^' unary)?   (right-associative)
    fn parse_power(&mut self) -> Option<Node> {
        let base = self.parse_atom()?;
        if matches!(self.peek(), Some(Tok::Caret)) {
            self.pos += 1;
            let exp = self.parse_unary()?;
            return Some(Node::Bin(Op::Pow, Box::new(base), Box::new(exp)));
        }
        Some(base)
    }

    fn parse_atom(&mut self) -> Option<Node> {
        match self.next()? {
            Tok::Num(n) => Some(Node::Num(n)),
            Tok::LParen => {
                let e = self.parse_expr()?;
                match self.next() {
                    Some(Tok::RParen) => Some(e),
                    _ => None,
                }
            }
            Tok::Ident(name) => {
                let lower = name.to_lowercase();
                // Constant?
                match lower.as_str() {
                    "x" => return Some(Node::Var),
                    "pi" | "π" => return Some(Node::Num(std::f64::consts::PI)),
                    "e" => return Some(Node::Num(std::f64::consts::E)),
                    "tau" => return Some(Node::Num(std::f64::consts::TAU)),
                    _ => {}
                }
                // Function call?
                let func = func_from_name(&lower)?;
                if !matches!(self.next(), Some(Tok::LParen)) {
                    return None;
                }
                let arg = self.parse_expr()?;
                if !matches!(self.next(), Some(Tok::RParen)) {
                    return None;
                }
                Some(Node::Call(func, Box::new(arg)))
            }
            _ => None,
        }
    }
}

fn func_from_name(name: &str) -> Option<Func> {
    Some(match name {
        "sin" => Func::Sin,
        "cos" => Func::Cos,
        "tan" => Func::Tan,
        "asin" | "arcsin" => Func::Asin,
        "acos" | "arccos" => Func::Acos,
        "atan" | "arctan" => Func::Atan,
        "sinh" => Func::Sinh,
        "cosh" => Func::Cosh,
        "tanh" => Func::Tanh,
        "exp" => Func::Exp,
        "ln" => Func::Ln,
        "log" | "log10" => Func::Log10,
        "log2" => Func::Log2,
        "sqrt" => Func::Sqrt,
        "cbrt" => Func::Cbrt,
        "abs" => Func::Abs,
        "floor" => Func::Floor,
        "ceil" => Func::Ceil,
        "round" => Func::Round,
        "sign" | "sgn" => Func::Sign,
        "gamma" => Func::Gamma,
        _ => return None,
    })
}

fn eval_node(n: &Node, x: f64) -> f64 {
    match n {
        Node::Num(v) => *v,
        Node::Var => x,
        Node::Neg(a) => -eval_node(a, x),
        Node::Bin(op, a, b) => {
            let (a, b) = (eval_node(a, x), eval_node(b, x));
            match op {
                Op::Add => a + b,
                Op::Sub => a - b,
                Op::Mul => a * b,
                Op::Div => a / b,
                Op::Pow => a.powf(b),
            }
        }
        Node::Call(f, a) => {
            let a = eval_node(a, x);
            match f {
                Func::Sin => a.sin(),
                Func::Cos => a.cos(),
                Func::Tan => a.tan(),
                Func::Asin => a.asin(),
                Func::Acos => a.acos(),
                Func::Atan => a.atan(),
                Func::Sinh => a.sinh(),
                Func::Cosh => a.cosh(),
                Func::Tanh => a.tanh(),
                Func::Exp => a.exp(),
                Func::Ln => a.ln(),
                Func::Log10 => a.log10(),
                Func::Log2 => a.log2(),
                Func::Sqrt => a.sqrt(),
                Func::Cbrt => a.cbrt(),
                Func::Abs => a.abs(),
                Func::Floor => a.floor(),
                Func::Ceil => a.ceil(),
                Func::Round => a.round(),
                Func::Sign => a.signum(),
                Func::Gamma => gamma(a),
            }
        }
    }
}

/// Lanczos Γ(x) (g=7), with the reflection formula for x < 0.5.
fn gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x))
    } else {
        let x = x - 1.0;
        let mut a = C[0];
        let t = x + G + 0.5;
        for (i, &c) in C.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        (2.0 * std::f64::consts::PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
    }
}

/// A sampled curve: `(x, Some(y))`, or `(x, None)` where the function is
/// undefined (so the renderer can lift the pen across the gap).
pub type Samples = Vec<(f64, Option<f64>)>;

/// Sample `expr` at `n` evenly spaced points over `[x0, x1]`.
pub fn sample(expr: &Expr, x0: f64, x1: f64, n: usize) -> Samples {
    let n = n.max(2);
    let step = (x1 - x0) / (n - 1) as f64;
    (0..n)
        .map(|i| {
            let x = x0 + step * i as f64;
            (x, expr.eval(x))
        })
        .collect()
}

/// Numeric intersections of `f` and `g` on `[x0, x1]`: scan for sign changes of
/// `f - g` across `n` sub-intervals and bisect each to a root. Returns `(x, y)`
/// pairs, deduplicated to `tol`.
pub fn intersections(f: &Expr, g: &Expr, x0: f64, x1: f64, n: usize) -> Vec<(f64, f64)> {
    let d = |x: f64| match (f.eval(x), g.eval(x)) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };
    let n = n.max(2);
    let step = (x1 - x0) / n as f64;
    let mut roots: Vec<(f64, f64)> = Vec::new();
    let mut prev_x = x0;
    let mut prev = d(x0);
    for i in 1..=n {
        let x = x0 + step * i as f64;
        let cur = d(x);
        if let (Some(a), Some(b)) = (prev, cur) {
            if a == 0.0 {
                push_root(&mut roots, f, prev_x);
            } else if a * b < 0.0 {
                // Bisect for the root in [prev_x, x].
                let (mut lo, mut hi, mut flo) = (prev_x, x, a);
                for _ in 0..60 {
                    let mid = 0.5 * (lo + hi);
                    match d(mid) {
                        Some(fm) => {
                            if flo * fm <= 0.0 {
                                hi = mid;
                            } else {
                                lo = mid;
                                flo = fm;
                            }
                        }
                        None => break,
                    }
                }
                push_root(&mut roots, f, 0.5 * (lo + hi));
            }
        }
        prev_x = x;
        prev = cur;
    }
    roots
}

fn push_root(roots: &mut Vec<(f64, f64)>, f: &Expr, x: f64) {
    let y = f.eval(x).unwrap_or(f64::NAN);
    if roots.iter().any(|(rx, _)| (rx - x).abs() < 1e-6) {
        return;
    }
    roots.push((x, y));
}

/// Signed area between `f` and `g` on `[a, b]` by the trapezoidal rule
/// (`∫ (f - g) dx`), skipping sub-intervals where either is undefined.
pub fn area_between(f: &Expr, g: &Expr, a: f64, b: f64, n: usize) -> f64 {
    let n = n.max(2);
    let step = (b - a) / n as f64;
    let h = |x: f64| match (f.eval(x), g.eval(x)) {
        (Some(fa), Some(ga)) => Some(fa - ga),
        _ => None,
    };
    let mut sum = 0.0;
    let mut prev = h(a);
    for i in 1..=n {
        let x = a + step * i as f64;
        let cur = h(x);
        if let (Some(p), Some(c)) = (prev, cur) {
            sum += 0.5 * (p + c) * step;
        }
        prev = cur;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn parses_and_evaluates_basic() {
        let e = Expr::parse("x^2 + 1").unwrap();
        assert_eq!(e.eval(3.0), Some(10.0));
        let s = Expr::parse("sin(x)").unwrap();
        assert!(approx(
            s.eval(std::f64::consts::PI / 2.0).unwrap(),
            1.0,
            1e-9
        ));
    }

    #[test]
    fn strips_definition_prefix() {
        let e = Expr::parse("f(x) = x / 3").unwrap();
        assert_eq!(e.eval(9.0), Some(3.0));
        let g = Expr::parse("y = 2*x").unwrap();
        assert_eq!(g.eval(4.0), Some(8.0));
    }

    #[test]
    fn constants_and_precedence() {
        let e = Expr::parse("2 + 3 * 4").unwrap();
        assert_eq!(e.eval(0.0), Some(14.0));
        let p = Expr::parse("2 ^ 3 ^ 2").unwrap(); // right-assoc => 2^9 = 512
        assert_eq!(p.eval(0.0), Some(512.0));
        let pi = Expr::parse("pi").unwrap();
        assert!(approx(pi.eval(0.0).unwrap(), std::f64::consts::PI, 1e-12));
    }

    #[test]
    fn undefined_is_none() {
        let e = Expr::parse("ln(x)").unwrap();
        assert_eq!(e.eval(-1.0), None);
        let d = Expr::parse("1/x").unwrap();
        assert_eq!(d.eval(0.0), None);
    }

    #[test]
    fn intersections_of_sin_and_line() {
        // sin(x) = x/3 has roots at 0 and ±~2.2789 on [-5, 5].
        let f = Expr::parse("sin(x)").unwrap();
        let g = Expr::parse("x/3").unwrap();
        let roots = intersections(&f, &g, -5.0, 5.0, 400);
        assert!(roots.iter().any(|(x, _)| approx(*x, 0.0, 1e-3)));
        assert!(roots.iter().any(|(x, _)| approx(*x, 2.2789, 1e-2)));
        assert!(roots.iter().any(|(x, _)| approx(*x, -2.2789, 1e-2)));
    }

    #[test]
    fn area_of_constant_gap() {
        // f - g = 2 on [0, 3] => area 6.
        let f = Expr::parse("x + 2").unwrap();
        let g = Expr::parse("x").unwrap();
        assert!(approx(area_between(&f, &g, 0.0, 3.0, 200), 6.0, 1e-6));
    }

    #[test]
    fn rejects_garbage() {
        assert!(Expr::parse("2 +").is_none());
        assert!(Expr::parse("sin(").is_none());
        assert!(Expr::parse("@#$").is_none());
    }
}
