//! `tptplan array math`: evaluate an arithmetic expression over named Zarr
//! arrays and write the result to a Zarr array.
//!
//! The expression is parsed here and turned into a lazy [`Expr`] graph from
//! the array crate. Nothing is read until the output is written, and the
//! output is written chunk by chunk, so memory use is bounded by the chunk
//! size rather than by the input size. Results are float64. Elements that are
//! fill-valued or NaN in an input become NaN in the output.
//!
//! Grammar (whitespace is ignored between tokens):
//!
//! ```text
//! expression := term (('+' | '-') term)*
//! term       := unary (('*' | '/') unary)*
//! unary      := '-' unary | primary
//! primary    := NUMBER | NAME | FUNC '(' NAME ')' | '(' expression ')'
//! NUMBER     := digits, optionally with '.' and digits, optionally with an
//!               exponent 'e' or 'E', an optional sign and digits
//! NAME       := letter or '_', then letters, digits or '_'; must be named by
//!               an --input NAME=PATH argument
//! FUNC       := anomaly | nino34 | nino34_relative
//! ```
//!
//! The functions take one input array of shape [time, lat, lon] with monthly
//! steps. `anomaly` is the monthly anomaly and keeps the shape. `nino34` and
//! `nino34_relative` are the Niño-3.4 indices of `tptplan array index`: they
//! have shape \[time\], and need the grid of the input (`--lat-start`,
//! `--lat-step`, `--lon-start`, `--lon-step`).
//!
//! Operators are IEEE: division by zero gives infinities or NaN. Unary minus
//! binds tighter than `*` and `/`, so `-a * b` means `(-a) * b`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use tpt_planetary_array::lazy::{Block, Expr, NodeOp, Region};
use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::netcdf3::{Format, NcType, Writer};
use tpt_planetary_array::ops::ArithOp;
use tpt_planetary_array::ops::climate;
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Selection, Slice, Store};

use crate::array_cmd::{default_chunks, reject_remote, split_location};
use crate::array_convert::{is_netcdf_path, zarr_format};
use crate::{EXIT_RUNTIME, EXIT_USAGE, Failure};

/// A binary operator in the expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

/// Parsed expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    Num(f64),
    Name(String),
    /// A function applied to a named input.
    Call(Func, String),
    Neg(Box<Ast>),
    Bin(Op, Box<Ast>, Box<Ast>),
}

/// A function of the expression language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    /// Monthly anomaly of a [time, lat, lon] field.
    Anomaly,
    /// The Niño-3.4 index (shape \[time\]).
    Nino34,
    /// The relative Niño-3.4 index (shape \[time\]).
    Nino34Relative,
}

impl Func {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "anomaly" => Some(Self::Anomaly),
            "nino34" => Some(Self::Nino34),
            "nino34_relative" => Some(Self::Nino34Relative),
            _ => None,
        }
    }

    /// True for the functions that need the input grid.
    fn needs_grid(self) -> bool {
        !matches!(self, Self::Anomaly)
    }
}

/// The grid of the latitude and longitude axes of the inputs: regular and
/// ascending, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSpec {
    pub lat_start: f64,
    pub lat_step: f64,
    pub lon_start: f64,
    pub lon_step: f64,
}

impl GridSpec {
    /// Checks that the starts are finite and the steps positive.
    pub fn validate(&self) -> Result<(), Failure> {
        if !(self.lat_start.is_finite() && self.lon_start.is_finite()) {
            return Err(Failure::new(
                EXIT_USAGE,
                "grid starts must be finite numbers".to_string(),
            ));
        }
        let positive = |v: f64| v > 0.0 && v.is_finite();
        if !(positive(self.lat_step) && positive(self.lon_step)) {
            return Err(Failure::new(
                EXIT_USAGE,
                "grid steps must be positive: the coordinates must be ascending".to_string(),
            ));
        }
        Ok(())
    }

    fn coords(start: f64, step: f64, n: u64) -> Vec<f64> {
        (0..n).map(|i| start + step * i as f64).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(Op),
    LParen,
    RParen,
}

/// A syntax error with the 1-based column where it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub column: usize,
    pub message: String,
}

fn syntax(column: usize, message: impl Into<String>) -> SyntaxError {
    SyntaxError {
        column,
        message: message.into(),
    }
}

fn tokenize(text: &str) -> Result<Vec<(usize, Token)>, SyntaxError> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let column = i + 1;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'+' => {
                tokens.push((column, Token::Op(Op::Add)));
                i += 1;
            }
            b'-' => {
                tokens.push((column, Token::Op(Op::Sub)));
                i += 1;
            }
            b'*' => {
                tokens.push((column, Token::Op(Op::Mul)));
                i += 1;
            }
            b'/' => {
                tokens.push((column, Token::Op(Op::Div)));
                i += 1;
            }
            b'(' => {
                tokens.push((column, Token::LParen));
                i += 1;
            }
            b')' => {
                tokens.push((column, Token::RParen));
                i += 1;
            }
            b'0'..=b'9' | b'.' => {
                let start = i;
                let mut seen_digit = false;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                    seen_digit = true;
                }
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                        seen_digit = true;
                    }
                }
                if !seen_digit {
                    return Err(syntax(column, "a lone '.' is not a number"));
                }
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    let mut j = i + 1;
                    if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j].is_ascii_digit() {
                        while j < bytes.len() && bytes[j].is_ascii_digit() {
                            j += 1;
                        }
                        i = j;
                    }
                }
                let literal = &text[start..i];
                let value: f64 = literal
                    .parse()
                    .map_err(|_| syntax(column, format!("invalid number '{literal}'")))?;
                tokens.push((column, Token::Num(value)));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                tokens.push((column, Token::Name(text[start..i].to_string())));
            }
            other => {
                return Err(syntax(
                    column,
                    format!("unexpected character '{}'", other as char),
                ));
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    pos: usize,
    end_column: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(_, t)| t)
    }

    fn here(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.end_column, |(c, _)| *c)
    }

    fn describe(token: Option<&Token>) -> String {
        match token {
            None => "end of expression".to_string(),
            Some(Token::Num(v)) => format!("number {v}"),
            Some(Token::Name(n)) => format!("name '{n}'"),
            Some(Token::Op(op)) => format!(
                "'{}'",
                match op {
                    Op::Add => '+',
                    Op::Sub => '-',
                    Op::Mul => '*',
                    Op::Div => '/',
                }
            ),
            Some(Token::LParen) => "'('".to_string(),
            Some(Token::RParen) => "')'".to_string(),
        }
    }

    fn expression(&mut self) -> Result<Ast, SyntaxError> {
        let mut lhs = self.term()?;
        while let Some(Token::Op(op @ (Op::Add | Op::Sub))) = self.peek() {
            let op = *op;
            self.pos += 1;
            let rhs = self.term()?;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Ast, SyntaxError> {
        let mut lhs = self.unary()?;
        while let Some(Token::Op(op @ (Op::Mul | Op::Div))) = self.peek() {
            let op = *op;
            self.pos += 1;
            let rhs = self.unary()?;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Ast, SyntaxError> {
        if let Some(Token::Op(Op::Sub)) = self.peek() {
            self.pos += 1;
            return Ok(Ast::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Ast, SyntaxError> {
        let column = self.here();
        match self.peek().cloned() {
            Some(Token::Num(v)) => {
                self.pos += 1;
                Ok(Ast::Num(v))
            }
            Some(Token::Name(n)) => {
                self.pos += 1;
                if !matches!(self.peek(), Some(Token::LParen)) {
                    return Ok(Ast::Name(n));
                }
                let func = Func::parse(&n).ok_or_else(|| {
                    syntax(
                        column,
                        format!(
                            "'{n}' is not a function; the functions are anomaly, nino34 and nino34_relative"
                        ),
                    )
                })?;
                self.pos += 1;
                let arg = match self.peek().cloned() {
                    Some(Token::Name(a)) => {
                        self.pos += 1;
                        a
                    }
                    other => {
                        return Err(syntax(
                            self.here(),
                            format!(
                                "expected an input name but found {}",
                                Self::describe(other.as_ref())
                            ),
                        ));
                    }
                };
                match self.peek() {
                    Some(Token::RParen) => {
                        self.pos += 1;
                        Ok(Ast::Call(func, arg))
                    }
                    other => Err(syntax(
                        self.here(),
                        format!("expected ')' but found {}", Self::describe(other)),
                    )),
                }
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.expression()?;
                match self.peek() {
                    Some(Token::RParen) => {
                        self.pos += 1;
                        Ok(inner)
                    }
                    other => Err(syntax(
                        self.here(),
                        format!("expected ')' but found {}", Self::describe(other)),
                    )),
                }
            }
            other => Err(syntax(
                column,
                format!(
                    "expected a number, a name or '(' but found {}",
                    Self::describe(other.as_ref())
                ),
            )),
        }
    }
}

/// Parses an expression into a tree. The whole input must be consumed.
pub fn parse(text: &str) -> Result<Ast, SyntaxError> {
    let tokens = tokenize(text)?;
    if tokens.is_empty() {
        return Err(syntax(1, "the expression is empty"));
    }
    let mut parser = Parser {
        tokens,
        pos: 0,
        end_column: text.len() + 1,
    };
    let ast = parser.expression()?;
    if parser.pos < parser.tokens.len() {
        return Err(syntax(
            parser.here(),
            format!(
                "unexpected {} after a complete expression",
                Parser::describe(parser.peek())
            ),
        ));
    }
    Ok(ast)
}

/// Names referenced by an expression, in order of first use.
fn names(ast: &Ast, out: &mut Vec<String>) {
    match ast {
        Ast::Num(_) => {}
        Ast::Name(n) => {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
        Ast::Call(_, arg) => {
            if !out.contains(arg) {
                out.push(arg.clone());
            }
        }
        Ast::Neg(inner) => names(inner, out),
        Ast::Bin(_, a, b) => {
            names(a, out);
            names(b, out);
        }
    }
}

/// A constant broadcast over `shape`, used for scalar operands of division
/// and of operations with a scalar on the left.
#[derive(Debug)]
struct Constant {
    shape: Vec<u64>,
    value: f64,
}

impl NodeOp for Constant {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> tpt_planetary_array::Result<Block> {
        let n: u64 = region.count.iter().product();
        let n = n as usize;
        Ok(Block {
            values: vec![self.value; n],
            valid: vec![true; n],
        })
    }
}

/// A value during evaluation: a scalar, or a lazy array.
#[derive(Clone)]
enum Value {
    Scalar(f64),
    Array(Expr),
}

fn runtime(message: String) -> Failure {
    Failure::new(EXIT_RUNTIME, message)
}

fn as_expr(value: &Value, like: &Expr) -> Expr {
    match value {
        Value::Array(e) => e.clone(),
        Value::Scalar(c) => Expr::from_op(Constant {
            shape: like.shape(),
            value: *c,
        }),
    }
}

fn combine(op: Op, lhs: Value, rhs: Value) -> Result<Value, Failure> {
    Ok(match (lhs, rhs) {
        (Value::Scalar(a), Value::Scalar(b)) => Value::Scalar(match op {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            Op::Div => a / b,
        }),
        // Array with scalar: add and multiply use the scalar node operations.
        // Subtraction of a scalar is addition of its negation (exact in IEEE).
        (Value::Array(a), Value::Scalar(c)) => match op {
            Op::Add => Value::Array(a.add_scalar(c)),
            Op::Sub => Value::Array(a.add_scalar(-c)),
            Op::Mul => Value::Array(a.mul_scalar(c)),
            Op::Div => {
                let k = as_expr(&Value::Scalar(c), &a);
                Value::Array(
                    a.arith(ArithOp::Div, &k)
                        .map_err(|e| runtime(e.to_string()))?,
                )
            }
        },
        // Scalar with array: c + a and c * a are commutative; c - a is
        // (-a) + c, which equals c - a exactly in IEEE arithmetic.
        (Value::Scalar(c), Value::Array(a)) => match op {
            Op::Add => Value::Array(a.add_scalar(c)),
            Op::Mul => Value::Array(a.mul_scalar(c)),
            Op::Sub => Value::Array(a.mul_scalar(-1.0).add_scalar(c)),
            Op::Div => {
                let k = as_expr(&Value::Scalar(c), &a);
                Value::Array(
                    k.arith(ArithOp::Div, &a)
                        .map_err(|e| runtime(e.to_string()))?,
                )
            }
        },
        (Value::Array(a), Value::Array(b)) => {
            let arith = match op {
                Op::Add => ArithOp::Add,
                Op::Sub => ArithOp::Sub,
                Op::Mul => ArithOp::Mul,
                Op::Div => ArithOp::Div,
            };
            Value::Array(a.arith(arith, &b).map_err(|e| runtime(e.to_string()))?)
        }
    })
}

fn evaluate(
    ast: &Ast,
    inputs: &BTreeMap<String, Expr>,
    grid: Option<&GridSpec>,
) -> Result<Value, Failure> {
    Ok(match ast {
        Ast::Call(func, arg) => {
            let field = inputs
                .get(arg)
                .ok_or_else(|| runtime(format!("no input named '{arg}'")))?;
            if func.needs_grid() && grid.is_none() {
                return Err(Failure::new(
                    EXIT_USAGE,
                    format!(
                        "{func:?} needs the grid of '{arg}': pass --lat-start, --lat-step, --lon-start and --lon-step"
                    ),
                ));
            }
            let expr = match func {
                Func::Anomaly => {
                    climate::monthly_anomaly(field).map_err(|e| runtime(format!("anomaly: {e}")))?
                }
                Func::Nino34 | Func::Nino34Relative => {
                    let grid = grid.expect("checked above");
                    let shape = field.shape();
                    if shape.len() != 3 {
                        return Err(runtime(format!(
                            "'{arg}' must have shape [time, lat, lon] for an index"
                        )));
                    }
                    let lat = GridSpec::coords(grid.lat_start, grid.lat_step, shape[1]);
                    let lon = GridSpec::coords(grid.lon_start, grid.lon_step, shape[2]);
                    let built = if *func == Func::Nino34 {
                        climate::nino34_expr(field, &lat, &lon)
                    } else {
                        climate::nino34_relative_expr(field, &lat, &lon)
                    };
                    built.map_err(|e| runtime(format!("{func:?}: {e}")))?
                }
            };
            Value::Array(expr)
        }
        Ast::Num(v) => Value::Scalar(*v),
        Ast::Name(n) => match inputs.get(n) {
            Some(e) => Value::Array(e.clone()),
            None => {
                return Err(runtime(format!(
                    "no input named '{n}': pass it as --input {n}=PATH"
                )));
            }
        },
        Ast::Neg(inner) => match evaluate(inner, inputs, grid)? {
            Value::Scalar(v) => Value::Scalar(-v),
            Value::Array(e) => Value::Array(e.mul_scalar(-1.0)),
        },
        Ast::Bin(op, a, b) => combine(*op, evaluate(a, inputs, grid)?, evaluate(b, inputs, grid)?)?,
    })
}

/// Opens each `NAME=PATH` input as a local Zarr array.
fn open_inputs(specs: &[String]) -> Result<BTreeMap<String, Expr>, Failure> {
    let mut out = BTreeMap::new();
    for spec in specs {
        let (name, path) = spec.split_once('=').ok_or_else(|| {
            Failure::new(
                EXIT_USAGE,
                format!("bad --input '{spec}': expected NAME=PATH"),
            )
        })?;
        let valid_name = !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid_name {
            return Err(Failure::new(
                EXIT_USAGE,
                format!("bad input name '{name}': use letters, digits and '_'"),
            ));
        }
        if out.contains_key(name) {
            return Err(Failure::new(
                EXIT_USAGE,
                format!("input '{name}' is given more than once"),
            ));
        }
        reject_remote(path)?;
        let (parent, node) = split_location(path)?;
        let store: Arc<dyn Store> = Arc::new(
            FsStore::open(&parent)
                .map_err(|e| runtime(format!("cannot open input '{name}': {e}")))?,
        );
        let array = Array::open(store, &node).map_err(|e| {
            runtime(format!(
                "cannot open input '{name}' from '{path}': {e} (expected a Zarr array directory)"
            ))
        })?;
        out.insert(name.to_string(), Expr::from_array(array));
    }
    Ok(out)
}

/// Writes `expr` as a float64 Zarr array at `output`, one chunk at a time.
fn write_result(expr: &Expr, output: &str, format: ZarrFormat) -> Result<u64, Failure> {
    let shape = expr.shape();
    let chunk = default_chunks(&shape);
    let meta = ArrayMetadata::new(format, shape.clone(), chunk.clone(), DType::FLOAT64)
        .map_err(|e| runtime(format!("cannot create output: {e}")))?
        .with_fill_value(FillValue::Float(f64::NAN));
    let (parent, name) = split_location(output)?;
    let store: Arc<dyn Store> = Arc::new(
        FsStore::open(&parent).map_err(|e| runtime(format!("cannot open output store: {e}")))?,
    );
    let array = Array::create(store, &name, meta)
        .map_err(|e| runtime(format!("cannot create output '{output}': {e}")))?;

    let grid: Vec<u64> = shape
        .iter()
        .zip(&chunk)
        .map(|(&n, &c)| n.div_ceil(c))
        .collect();
    let total_chunks: u64 = grid.iter().product();
    let mut written = 0u64;
    let mut coord = vec![0u64; shape.len()];
    for _ in 0..total_chunks {
        let start: Vec<u64> = coord.iter().zip(&chunk).map(|(&i, &c)| i * c).collect();
        let count: Vec<u64> = start
            .iter()
            .zip(&chunk)
            .zip(&shape)
            .map(|((&s, &c), &n)| c.min(n - s))
            .collect();
        let region = Region {
            start: start.clone(),
            count: count.clone(),
            step: vec![1; shape.len()],
        };
        let block = expr
            .eval_region(&region)
            .map_err(|e| runtime(format!("evaluation failed: {e}")))?;
        let values: Vec<f64> = block
            .values
            .iter()
            .zip(&block.valid)
            .map(|(&v, &ok)| if ok { v } else { f64::NAN })
            .collect();
        let selection: Vec<Selection> = start
            .iter()
            .zip(&count)
            .map(|(&s, &c)| Selection::Range(Slice::range(s as i64, (s + c) as i64)))
            .collect();
        array
            .write_f64(&selection, &values)
            .map_err(|e| runtime(format!("cannot write output '{output}': {e}")))?;
        written += 1;
        // Advance the chunk coordinate, last axis fastest.
        for axis in (0..coord.len()).rev() {
            coord[axis] += 1;
            if coord[axis] < grid[axis] {
                break;
            }
            coord[axis] = 0;
        }
    }
    Ok(written)
}

/// Writes `expr` as a NetCDF-3 (classic) file at `output`, with one float64
/// variable `result` and dimensions `dim0`, `dim1`, and so on. NetCDF-3 holds
/// each variable in one piece, so the whole result is evaluated in memory.
fn write_result_netcdf(expr: &Expr, output: &str) -> Result<u64, Failure> {
    let shape = expr.shape();
    let whole = Region {
        start: vec![0; shape.len()],
        count: shape.clone(),
        step: vec![1; shape.len()],
    };
    let block = expr
        .eval_region(&whole)
        .map_err(|e| runtime(format!("evaluation failed: {e}")))?;
    let values: Vec<f64> = block
        .values
        .iter()
        .zip(&block.valid)
        .map(|(&v, &ok)| if ok { v } else { f64::NAN })
        .collect();
    let mut writer = Writer::new(Format::Classic);
    let mut dims = Vec::with_capacity(shape.len());
    for (i, &n) in shape.iter().enumerate() {
        let len = usize::try_from(n)
            .map_err(|_| runtime(format!("axis {i} is too long for a NetCDF file")))?;
        let id = writer
            .add_dimension(&format!("dim{i}"), Some(len))
            .map_err(|e| runtime(format!("cannot create dimension: {e}")))?;
        dims.push(id);
    }
    writer
        .add_variable_f64("result", &dims, Vec::new(), NcType::Double, &values)
        .map_err(|e| runtime(format!("cannot create variable: {e}")))?;
    let bytes = writer
        .finish()
        .map_err(|e| runtime(format!("cannot serialise NetCDF: {e}")))?;
    std::fs::write(output, bytes).map_err(|e| runtime(format!("cannot write '{output}': {e}")))?;
    Ok(values.len() as u64)
}

/// Evaluates `expression` over the named inputs and writes a Zarr array, or a
/// NetCDF-3 file when the output name ends in `.nc`.
/// Returns the summary line for stdout.
pub fn math(
    expression: &str,
    inputs: &[String],
    output: &str,
    version: u8,
    grid: Option<GridSpec>,
    verbose: u8,
) -> Result<String, Failure> {
    let ast = parse(expression).map_err(|e| {
        Failure::new(
            EXIT_USAGE,
            format!("syntax error at column {}: {}", e.column, e.message),
        )
    })?;
    reject_remote(output)?;
    let to_netcdf = is_netcdf_path(output);
    let mut used = Vec::new();
    names(&ast, &mut used);
    let opened = open_inputs(inputs)?;
    if let Some(missing) = used.iter().find(|n| !opened.contains_key(*n)) {
        return Err(runtime(format!(
            "no input named '{missing}': pass it as --input {missing}=PATH"
        )));
    }
    if Path::new(output).exists() {
        return Err(runtime(format!(
            "output '{output}' already exists; remove it or choose another path"
        )));
    }
    if let Some(g) = &grid {
        g.validate()?;
    }
    let result = match evaluate(&ast, &opened, grid.as_ref())? {
        Value::Scalar(_) => {
            return Err(Failure::new(
                EXIT_USAGE,
                "the expression has no array operand; reference at least one --input name"
                    .to_string(),
            ));
        }
        Value::Array(e) => e,
    };
    if verbose > 0 {
        eprintln!("tptplan: evaluating {:?} over {:?}", result.shape(), used);
    }
    if to_netcdf {
        let values = write_result_netcdf(&result, output)?;
        return Ok(format!(
            "wrote {} with shape {:?} ({values} values, NetCDF-3)",
            output,
            result.shape()
        ));
    }
    let chunks = write_result(&result, output, zarr_format(version))?;
    Ok(format!(
        "wrote {} with shape {:?} in {chunks} chunk{}",
        output,
        result.shape(),
        if chunks == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_unary_minus() {
        assert_eq!(
            parse("1 + 2 * 3").unwrap(),
            Ast::Bin(
                Op::Add,
                Box::new(Ast::Num(1.0)),
                Box::new(Ast::Bin(
                    Op::Mul,
                    Box::new(Ast::Num(2.0)),
                    Box::new(Ast::Num(3.0))
                ))
            )
        );
        assert_eq!(
            parse("-a * b").unwrap(),
            Ast::Bin(
                Op::Mul,
                Box::new(Ast::Neg(Box::new(Ast::Name("a".into())))),
                Box::new(Ast::Name("b".into()))
            )
        );
        assert_eq!(parse("(1)").unwrap(), Ast::Num(1.0));
        assert_eq!(parse("1e3").unwrap(), Ast::Num(1000.0));
        assert_eq!(parse(".5").unwrap(), Ast::Num(0.5));
    }

    #[test]
    fn syntax_errors_report_columns() {
        let e = parse("a +").unwrap_err();
        assert_eq!(e.column, 4);
        let e = parse("a $ b").unwrap_err();
        assert_eq!(e.column, 3);
        let e = parse("(a").unwrap_err();
        assert!(e.message.contains("expected ')'"));
        let e = parse("a b").unwrap_err();
        assert!(e.message.contains("after a complete expression"));
        assert!(parse("").is_err());
        assert!(parse("1.2.3").is_err());
    }

    #[test]
    fn names_are_collected_once_in_order() {
        let ast = parse("b * a + b - (c)").unwrap();
        let mut out = Vec::new();
        names(&ast, &mut out);
        assert_eq!(out, vec!["b", "a", "c"]);
    }

    #[test]
    fn scalar_subexpressions_fold() {
        let v = combine(Op::Div, Value::Scalar(1.0), Value::Scalar(4.0)).unwrap();
        assert!(matches!(v, Value::Scalar(x) if x == 0.25));
    }
}
