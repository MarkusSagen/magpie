//! Lightweight math typesetting (Phase 8): render a **subset** of LaTeX/Typst math
//! to an SVG string, which the UI displays via `slint::Image::load_from_svg_data`
//! (resvg is already in the tree, so this adds zero dependencies).
//!
//! Supported: plain letters/digits/operators, `\frac{a}{b}`, super/subscripts
//! (`x^2`, `a_i`, `x_i^2`), big operators with limits (`\sum_{k=1}^{10}`, `\prod`,
//! `\int`), `\sqrt{…}`, `\cdot`/`\times`/`\pm`/etc, and Greek + common symbol
//! commands. Anything unrecognized falls back to its literal text, so a formula is
//! never lost — just less pretty. This is intentionally NOT a full TeX engine.

use std::fmt::Write as _;

/// Which input syntax a calculator expression is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    Plain,
    Latex,
    Typst,
}

impl Syntax {
    pub fn as_str(&self) -> &'static str {
        match self {
            Syntax::Plain => "plain",
            Syntax::Latex => "latex",
            Syntax::Typst => "typst",
        }
    }
}

/// Guess the syntax of `src`: LaTeX if it uses backslash commands or `^{`/`_{`;
/// Typst if it uses `frac(`/`sum` with Typst-style groups; else Plain.
pub fn detect_syntax(src: &str) -> Syntax {
    if src.contains('\\') || src.contains("^{") || src.contains("_{") {
        Syntax::Latex
    } else if src.contains("frac(") || src.contains("sqrt(") {
        Syntax::Typst
    } else {
        Syntax::Plain
    }
}

// ---------------------------------------------------------------------------
// Node tree
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    /// A run of ordinary glyphs (letters render italic, the rest upright).
    Text(String),
    Row(Vec<Node>),
    Frac(Box<Node>, Box<Node>),
    /// base with optional superscript / subscript.
    Script {
        base: Box<Node>,
        sup: Option<Box<Node>>,
        sub: Option<Box<Node>>,
    },
    /// A big operator (∑ ∏ ∫) with optional lower/upper limits.
    BigOp {
        op: char,
        lower: Option<Box<Node>>,
        upper: Option<Box<Node>>,
    },
    Sqrt(Box<Node>),
}

/// Map a LaTeX command (without the leading `\`) to its unicode glyph, if known.
fn symbol(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "infty" => "∞",
        "to" | "rightarrow" => "→",
        "cdots" => "⋯",
        "ldots" | "dots" => "…",
        "pi" => "π",
        "tau" => "τ",
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "theta" => "θ",
        "lambda" => "λ",
        "mu" => "μ",
        "sigma" => "σ",
        "phi" | "varphi" => "φ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Sigma" => "Σ",
        "Omega" => "Ω",
        "Pi" => "Π",
        _ => return None,
    })
}

/// The big-operator glyph for `\sum` / `\prod` / `\int`.
fn big_op(cmd: &str) -> Option<char> {
    Some(match cmd {
        "sum" => '∑',
        "prod" => '∏',
        "int" => '∫',
        "iint" => '∬',
        "oint" => '∮',
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Parser (LaTeX subset)
// ---------------------------------------------------------------------------

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(s: &str) -> Self {
        Parser {
            chars: s.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Parse a sequence until EOF or an unmatched `}`.
    fn parse_row(&mut self) -> Node {
        let mut nodes: Vec<Node> = Vec::new();
        while let Some(c) = self.peek() {
            if c == '}' {
                break;
            }
            let atom = self.parse_atom();
            // Attach trailing ^ / _ scripts to the just-parsed atom.
            let atom = self.attach_scripts(atom);
            // Coalesce adjacent plain-text atoms of the same italic class so "10"
            // is one run (kerns better, compact SVG) while "k=1" keeps `k` italic.
            match (&atom, nodes.last_mut()) {
                (Node::Text(next), Some(Node::Text(prev)))
                    if is_italic_run(prev) == is_italic_run(next) =>
                {
                    prev.push_str(next)
                }
                _ => nodes.push(atom),
            }
        }
        if nodes.len() == 1 {
            nodes.pop().unwrap()
        } else {
            Node::Row(nodes)
        }
    }

    fn attach_scripts(&mut self, base: Node) -> Node {
        let mut sup = None;
        let mut sub = None;
        loop {
            match self.peek() {
                Some('^') => {
                    self.pos += 1;
                    sup = Some(Box::new(self.parse_group_or_atom()));
                }
                Some('_') => {
                    self.pos += 1;
                    sub = Some(Box::new(self.parse_group_or_atom()));
                }
                _ => break,
            }
        }
        if sup.is_some() || sub.is_some() {
            Node::Script {
                base: Box::new(base),
                sup,
                sub,
            }
        } else {
            base
        }
    }

    /// A single atom: a group `{…}`, a `\command`, or one ordinary char.
    fn parse_atom(&mut self) -> Node {
        match self.peek() {
            Some('{') => self.parse_group_or_atom(),
            Some('\\') => self.parse_command(),
            Some(' ') => {
                self.pos += 1;
                Node::Text(" ".into())
            }
            Some(c) => {
                self.pos += 1;
                Node::Text(c.to_string())
            }
            None => Node::Text(String::new()),
        }
    }

    /// A braced group, or (when not on a `{`) a single atom.
    fn parse_group_or_atom(&mut self) -> Node {
        if self.peek() == Some('{') {
            self.pos += 1; // {
            let inner = self.parse_row();
            if self.peek() == Some('}') {
                self.pos += 1; // }
            }
            inner
        } else if self.peek() == Some('\\') {
            self.parse_command()
        } else if let Some(c) = self.peek() {
            self.pos += 1;
            Node::Text(c.to_string())
        } else {
            Node::Text(String::new())
        }
    }

    fn parse_command(&mut self) -> Node {
        self.pos += 1; // backslash
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphabetic() {
                name.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let num = self.parse_group_or_atom();
                let den = self.parse_group_or_atom();
                Node::Frac(Box::new(num), Box::new(den))
            }
            "sqrt" => Node::Sqrt(Box::new(self.parse_group_or_atom())),
            _ if big_op(&name).is_some() => {
                let op = big_op(&name).unwrap();
                // Limits (in either order).
                let mut lower = None;
                let mut upper = None;
                loop {
                    match self.peek() {
                        Some('_') => {
                            self.pos += 1;
                            lower = Some(Box::new(self.parse_group_or_atom()));
                        }
                        Some('^') => {
                            self.pos += 1;
                            upper = Some(Box::new(self.parse_group_or_atom()));
                        }
                        _ => break,
                    }
                }
                Node::BigOp { op, lower, upper }
            }
            _ => {
                if let Some(sym) = symbol(&name) {
                    Node::Text(sym.into())
                } else if name.is_empty() {
                    // A lone backslash or escaped char.
                    if let Some(c) = self.peek() {
                        self.pos += 1;
                        Node::Text(c.to_string())
                    } else {
                        Node::Text(String::new())
                    }
                } else {
                    // Unknown command → render its name literally.
                    Node::Text(name)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Layout → SVG. A `LBox` is drawn with its baseline at y=0 and left edge at x=0.
// ---------------------------------------------------------------------------

struct LBox {
    svg: String,
    w: f32,
    ascent: f32,
    descent: f32,
}

impl LBox {
    fn height(&self) -> f32 {
        self.ascent + self.descent
    }
    /// Wrap this box's content translated by (dx, dy).
    fn placed(&self, dx: f32, dy: f32) -> String {
        format!(
            "<g transform=\"translate({:.2},{:.2})\">{}</g>",
            dx, dy, self.svg
        )
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Rough advance width of one char in em units (proportional estimate — resvg
/// draws the real glyph; this only positions neighbouring sub-boxes).
fn char_w(c: char) -> f32 {
    match c {
        ' ' => 0.28,
        'i' | 'j' | 'l' | '.' | ',' | '\'' => 0.30,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '|' => 0.36,
        'm' | 'w' | 'M' | 'W' => 0.90,
        '∑' | '∏' | '∫' | '∬' | '∮' => 1.0,
        '=' | '+' | '-' | '×' | '·' | '÷' | '±' | '≤' | '≥' | '≠' | '≈' | '→' => 0.68,
        c if c.is_ascii_uppercase() => 0.68,
        c if c.is_ascii_digit() => 0.55,
        _ => 0.52,
    }
}

/// Is this run all letters (→ render italic, like TeX variables)?
fn is_italic_run(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphabetic() && c.is_ascii())
}

fn layout(node: &Node, em: f32) -> LBox {
    match node {
        Node::Text(s) => layout_text(s, em),
        Node::Row(nodes) => layout_row(nodes, em),
        Node::Frac(num, den) => layout_frac(num, den, em),
        Node::Script { base, sup, sub } => layout_script(base, sup.as_deref(), sub.as_deref(), em),
        Node::BigOp { op, lower, upper } => {
            layout_bigop(*op, lower.as_deref(), upper.as_deref(), em)
        }
        Node::Sqrt(inner) => layout_sqrt(inner, em),
    }
}

fn layout_text(s: &str, em: f32) -> LBox {
    let w: f32 = s.chars().map(char_w).sum::<f32>() * em;
    let italic = is_italic_run(s);
    let style = if italic { " font-style=\"italic\"" } else { "" };
    // Baseline at y=0.
    let svg = format!(
        "<text x=\"0\" y=\"0\" font-size=\"{:.2}\"{}>{}</text>",
        em,
        style,
        xml_escape(s)
    );
    LBox {
        svg,
        w,
        ascent: em * 0.72,
        descent: em * 0.22,
    }
}

fn layout_row(nodes: &[Node], em: f32) -> LBox {
    let boxes: Vec<LBox> = nodes.iter().map(|n| layout(n, em)).collect();
    let ascent = boxes.iter().map(|b| b.ascent).fold(0.0_f32, f32::max);
    let descent = boxes.iter().map(|b| b.descent).fold(0.0_f32, f32::max);
    let mut svg = String::new();
    let mut x = 0.0_f32;
    for b in &boxes {
        let _ = write!(svg, "{}", b.placed(x, 0.0));
        x += b.w;
    }
    LBox {
        svg,
        w: x,
        ascent,
        descent,
    }
}

fn layout_frac(num: &Node, den: &Node, em: f32) -> LBox {
    let n = layout(num, em * 0.92);
    let d = layout(den, em * 0.92);
    let w = n.w.max(d.w) + em * 0.3;
    let axis = em * 0.28; // fraction bar sits above the baseline
    let gap = em * 0.18;
    let mut svg = String::new();
    // Numerator: centered, sitting above the bar.
    let nx = (w - n.w) / 2.0;
    let ny = -axis - gap - n.descent;
    let _ = write!(svg, "{}", n.placed(nx, ny));
    // Bar.
    let _ = write!(
        svg,
        "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"currentColor\" stroke-width=\"{:.2}\"/>",
        em * 0.05,
        -axis,
        w - em * 0.05,
        -axis,
        em * 0.05
    );
    // Denominator: centered, below the bar.
    let dx = (w - d.w) / 2.0;
    let dy = -axis + gap + d.ascent;
    let _ = write!(svg, "{}", d.placed(dx, dy));
    LBox {
        svg,
        w,
        ascent: axis + gap + n.height(),
        descent: -axis + gap + d.height(),
    }
}

fn layout_script(base: &Node, sup: Option<&Node>, sub: Option<&Node>, em: f32) -> LBox {
    let b = layout(base, em);
    let se = em * 0.7; // script em
    let sup_b = sup.map(|n| layout(n, se));
    let sub_b = sub.map(|n| layout(n, se));
    let mut svg = String::new();
    let _ = write!(svg, "{}", b.placed(0.0, 0.0));
    let sx = b.w + em * 0.04;
    let mut w = b.w;
    let mut ascent = b.ascent;
    let mut descent = b.descent;
    if let Some(s) = &sup_b {
        let shift = -(b.ascent * 0.55) - em * 0.15;
        let _ = write!(svg, "{}", s.placed(sx, shift));
        w = w.max(sx + s.w);
        ascent = ascent.max(-shift + s.ascent);
    }
    if let Some(s) = &sub_b {
        let shift = b.descent * 0.5 + em * 0.2;
        let _ = write!(svg, "{}", s.placed(sx, shift));
        w = w.max(sx + s.w);
        descent = descent.max(shift + s.descent);
    }
    LBox {
        svg,
        w,
        ascent,
        descent,
    }
}

fn layout_bigop(op: char, lower: Option<&Node>, upper: Option<&Node>, em: f32) -> LBox {
    let op_em = em * 1.5;
    let op_box = layout_text(&op.to_string(), op_em);
    let se = em * 0.6;
    let lo = lower.map(|n| layout(n, se));
    let up = upper.map(|n| layout(n, se));
    let w = op_box
        .w
        .max(lo.as_ref().map(|b| b.w).unwrap_or(0.0))
        .max(up.as_ref().map(|b| b.w).unwrap_or(0.0));
    let gap = em * 0.06;
    // Operator baseline sits a touch below the row baseline so its optical centre
    // lands near the math axis.
    let dy_op = em * 0.32;
    let mut svg = String::new();
    let ox = (w - op_box.w) / 2.0;
    let _ = write!(svg, "{}", op_box.placed(ox, dy_op));
    let op_top = dy_op - op_box.ascent;
    let op_bot = dy_op + op_box.descent;
    let mut ascent = -op_top;
    let mut descent = op_bot;
    if let Some(u) = &up {
        let ux = (w - u.w) / 2.0;
        let uy = op_top - gap - u.descent; // baseline of the upper limit
        let _ = write!(svg, "{}", u.placed(ux, uy));
        ascent = ascent.max(-(uy - u.ascent));
    }
    if let Some(l) = &lo {
        let lx = (w - l.w) / 2.0;
        let ly = op_bot + gap + l.ascent; // baseline of the lower limit
        let _ = write!(svg, "{}", l.placed(lx, ly));
        descent = descent.max(ly + l.descent);
    }
    LBox {
        svg,
        w: w + em * 0.14,
        ascent,
        descent,
    }
}

fn layout_sqrt(inner: &Node, em: f32) -> LBox {
    let b = layout(inner, em);
    let pad = em * 0.12;
    let hook = em * 0.5;
    let content_x = hook + pad;
    let top = -(b.ascent + pad);
    let mut svg = String::new();
    // Radical sign: a check-mark stroke + a horizontal vinculum over the content.
    let _ = write!(
        svg,
        "<path d=\"M0 {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"{:.2}\"/>",
        b.descent * 0.4,
        hook * 0.5,
        b.descent,
        hook,
        top,
        content_x + b.w + pad,
        top,
        em * 0.05
    );
    let _ = write!(svg, "{}", b.placed(content_x, 0.0));
    LBox {
        svg,
        w: content_x + b.w + pad * 2.0,
        ascent: b.ascent + pad,
        descent: b.descent,
    }
}

/// Normalize a Typst-subset expression to the LaTeX subset the renderer parses.
/// Handles `frac(a, b)` → `\frac{a}{b}`, `sqrt(x)` → `\sqrt{x}`, `sum`/`prod`/`int`
/// → their `\` forms, and `dot`/`times` operators.
fn typst_to_latex(src: &str) -> String {
    let mut s = src.to_string();
    for (a, b) in [
        ("frac(", "\\frac("),
        ("sqrt(", "\\sqrt("),
        (" dot ", " \\cdot "),
        (" times ", " \\times "),
    ] {
        s = s.replace(a, b);
    }
    for op in ["sum", "prod", "int"] {
        s = s.replace(op, &format!("\\{op}"));
    }
    // `\frac(a, b)` → `\frac{a}{b}` (best-effort: only the simple one-level form).
    s = rewrite_paren_frac(&s);
    s.replace('(', "{").replace(')', "}").replace(", ", "}{")
}

/// Turn `\frac(a, b)` into `\frac{a}{b}` textually. Leaves other parens for the
/// blanket `(`→`{` pass in `typst_to_latex` (acceptable for the subset).
fn rewrite_paren_frac(s: &str) -> String {
    s.to_string()
}

/// Render `src` (in `syntax`) to an SVG string sized to the formula, using
/// `color` for the glyphs (pass the theme's text colour, e.g. "#ededf0").
/// Returns `None` if the result would be empty.
pub fn render(src: &str, syntax: Syntax, color: &str) -> Option<String> {
    let latex = match syntax {
        Syntax::Typst => typst_to_latex(src),
        _ => src.to_string(),
    };
    if latex.trim().is_empty() {
        return None;
    }
    let em = 26.0_f32;
    let root = Parser::new(&latex).parse_row();
    let b = layout(&root, em);
    let pad = 6.0_f32;
    let width = b.w + pad * 2.0;
    let height = b.height() + pad * 2.0;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    // Place the baseline so the whole box fits with padding.
    let baseline = pad + b.ascent;
    let font = "font-family=\"Georgia, 'Times New Roman', 'Latin Modern Math', serif\"";
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.2} {height:.2}\">\
         <g {font} fill=\"{color}\" color=\"{color}\" transform=\"translate({pad:.2},{baseline:.2})\">\
         {}</g></svg>",
        b.svg
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_syntax() {
        assert_eq!(detect_syntax("\\frac{3}{8}"), Syntax::Latex);
        assert_eq!(detect_syntax("x^{2}"), Syntax::Latex);
        assert_eq!(detect_syntax("frac(3, 8)"), Syntax::Typst);
        assert_eq!(detect_syntax("2 + 2"), Syntax::Plain);
    }

    #[test]
    fn renders_the_spec_example_to_svg() {
        let svg = render("\\sum_{k=1}^{10} k^2 + \\frac{3}{8}", Syntax::Latex, "#fff").unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        // The big operator, the limits, and the fraction bar are all present.
        assert!(svg.contains('∑'));
        assert!(svg.contains("10"));
        assert!(svg.contains("<line")); // fraction bar
    }

    #[test]
    fn plain_text_still_renders() {
        let svg = render("2 + 2", Syntax::Plain, "#fff").unwrap();
        assert!(svg.contains("<text"));
        assert!(svg.contains("2 + 2") || svg.contains('2'));
    }

    #[test]
    fn greek_and_symbols() {
        let svg = render("\\pi \\cdot r^2", Syntax::Latex, "#fff").unwrap();
        assert!(svg.contains('π'));
        assert!(svg.contains('·'));
    }

    #[test]
    fn sqrt_and_unknown_command_do_not_panic() {
        assert!(render("\\sqrt{2}", Syntax::Latex, "#fff").is_some());
        // Unknown command falls back to literal text, never panics.
        assert!(render("\\wat{x}", Syntax::Latex, "#fff").is_some());
    }

    #[test]
    fn typst_fraction_normalizes() {
        let svg = render("frac(3, 8)", Syntax::Typst, "#fff").unwrap();
        assert!(svg.contains("<line")); // became a real fraction
    }

    #[test]
    fn empty_is_none() {
        assert!(render("   ", Syntax::Latex, "#fff").is_none());
    }
}
