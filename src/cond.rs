//! Transition conditions: SCXML `cond` under decree's custom data model (spec section 5, Rules).
//!
//! A `cond` is exactly one comparison, `<operand> <op> <operand>`. Operands are integer
//! literals, double-quoted strings, `true`, `false`, `data.<name>` and `visits.<state>`;
//! operators are `==` `!=` `<` `<=` `>` `>=`. There is no `&&`, `||` or parentheses.
//!
//! The spec defines no type coercion, so evaluation refuses to compare values of different
//! types, and the ordering operators apply to integers only.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// A parsed `cond`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cond {
    pub left: Operand,
    pub op: Op,
    pub right: Operand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Int(i64),
    Str(String),
    Bool(bool),
    /// `data.<name>`
    Data(String),
    /// `visits.<state>`
    Visits(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A value a `cond` compares: a literal, a `data` value, or a visit count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CondError {
    #[error("empty cond: expected `<operand> <op> <operand>`")]
    Empty,
    #[error("unexpected `{0}`: a cond is one comparison, without `&&`, `||` or parentheses")]
    UnexpectedChar(char),
    #[error("unterminated string literal")]
    UnterminatedString,
    #[error("`\\` in a string literal: the cond grammar has no escapes")]
    StringEscape,
    #[error("unknown operand `{0}`: expected an integer, a double-quoted string, `true`, `false`, `data.<name>` or `visits.<state>`")]
    UnknownOperand(String),
    #[error("integer literal `{0}` is out of range")]
    IntOutOfRange(String),
    #[error("expected exactly one comparison `<operand> <op> <operand>`, got: {0}")]
    Shape(String),
    #[error("unknown data `{0}`")]
    UnknownData(String),
    #[error("cannot compare {left} with {right}")]
    TypeMismatch {
        left: &'static str,
        right: &'static str,
    },
    #[error("`{op}` compares integers only, not {kind}")]
    NotOrdered { op: Op, kind: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Str(String),
    Op(Op),
    Word(String),
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Token::Str(s) => format!("\"{s}\""),
            Token::Op(op) => op.to_string(),
            Token::Word(w) => w.clone(),
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
        })
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Int(n) => write!(f, "{n}"),
            Operand::Str(s) => write!(f, "\"{s}\""),
            Operand::Bool(b) => write!(f, "{b}"),
            Operand::Data(name) => write!(f, "data.{name}"),
            Operand::Visits(state) => write!(f, "visits.{state}"),
        }
    }
}

impl fmt::Display for Cond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.left, self.op, self.right)
    }
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "an int",
            Value::Str(_) => "a string",
            Value::Bool(_) => "a bool",
        }
    }
}

impl FromStr for Cond {
    type Err = CondError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse(s)
    }
}

/// Parse a `cond` string.
pub fn parse(input: &str) -> Result<Cond, CondError> {
    let tokens = tokenize(input)?;
    match tokens.as_slice() {
        [] => Err(CondError::Empty),
        [left, Token::Op(op), right] => Ok(Cond {
            left: operand(left)?,
            op: *op,
            right: operand(right)?,
        }),
        _ => Err(CondError::Shape(
            tokens
                .iter()
                .map(Token::describe)
                .collect::<Vec<_>>()
                .join(" "),
        )),
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>, CondError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            loop {
                match chars.next() {
                    None => return Err(CondError::UnterminatedString),
                    Some('"') => break,
                    Some('\\') => return Err(CondError::StringEscape),
                    Some(ch) => s.push(ch),
                }
            }
            tokens.push(Token::Str(s));
        } else if matches!(c, '=' | '!' | '<' | '>') {
            chars.next();
            let eq = chars.next_if_eq(&'=').is_some();
            let op = match (c, eq) {
                ('=', true) => Op::Eq,
                ('!', true) => Op::Ne,
                ('<', false) => Op::Lt,
                ('<', true) => Op::Le,
                ('>', false) => Op::Gt,
                ('>', true) => Op::Ge,
                _ => return Err(CondError::UnexpectedChar(c)),
            };
            tokens.push(Token::Op(op));
        } else if is_word_char(c) {
            let mut w = String::new();
            while let Some(ch) = chars.next_if(|&ch| is_word_char(ch)) {
                w.push(ch);
            }
            tokens.push(Token::Word(w));
        } else {
            return Err(CondError::UnexpectedChar(c));
        }
    }
    Ok(tokens)
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}

fn operand(token: &Token) -> Result<Operand, CondError> {
    let word = match token {
        Token::Str(s) => return Ok(Operand::Str(s.clone())),
        Token::Op(op) => return Err(CondError::UnknownOperand(op.to_string())),
        Token::Word(w) => w.as_str(),
    };
    match word {
        "true" => return Ok(Operand::Bool(true)),
        "false" => return Ok(Operand::Bool(false)),
        _ => {}
    }
    let digits = word.strip_prefix('-').unwrap_or(word);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return word
            .parse()
            .map(Operand::Int)
            .map_err(|_| CondError::IntOutOfRange(word.to_string()));
    }
    if let Some(name) = word.strip_prefix("data.").filter(|n| is_ident(n)) {
        return Ok(Operand::Data(name.to_string()));
    }
    if let Some(state) = word.strip_prefix("visits.").filter(|n| is_ident(n)) {
        return Ok(Operand::Visits(state.to_string()));
    }
    Err(CondError::UnknownOperand(word.to_string()))
}

/// `^[a-z][a-z0-9_]*$`, the pattern for machine names, state ids and script names.
pub(crate) fn is_ident(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Cond {
    /// Names this cond reads from `data`, for validation (V10).
    pub fn data_refs(&self) -> impl Iterator<Item = &str> {
        [&self.left, &self.right]
            .into_iter()
            .filter_map(|o| match o {
                Operand::Data(name) => Some(name.as_str()),
                _ => None,
            })
    }

    /// States this cond reads from `visits`, for validation (V10).
    pub fn visits_refs(&self) -> impl Iterator<Item = &str> {
        [&self.left, &self.right]
            .into_iter()
            .filter_map(|o| match o {
                Operand::Visits(state) => Some(state.as_str()),
                _ => None,
            })
    }

    /// Evaluate against the run's `data` and visit counts. A state missing from `visits`
    /// has not been entered, so its count is 0.
    pub fn eval(
        &self,
        data: &BTreeMap<String, Value>,
        visits: &BTreeMap<String, u32>,
    ) -> Result<bool, CondError> {
        let left = resolve(&self.left, data, visits)?;
        let right = resolve(&self.right, data, visits)?;
        match (&left, &right) {
            (Value::Int(a), Value::Int(b)) => Ok(match self.op {
                Op::Eq => a == b,
                Op::Ne => a != b,
                Op::Lt => a < b,
                Op::Le => a <= b,
                Op::Gt => a > b,
                Op::Ge => a >= b,
            }),
            (Value::Str(_), Value::Str(_)) | (Value::Bool(_), Value::Bool(_)) => match self.op {
                Op::Eq => Ok(left == right),
                Op::Ne => Ok(left != right),
                op => Err(CondError::NotOrdered {
                    op,
                    kind: left.kind(),
                }),
            },
            _ => Err(CondError::TypeMismatch {
                left: left.kind(),
                right: right.kind(),
            }),
        }
    }
}

fn resolve(
    operand: &Operand,
    data: &BTreeMap<String, Value>,
    visits: &BTreeMap<String, u32>,
) -> Result<Value, CondError> {
    Ok(match operand {
        Operand::Int(n) => Value::Int(*n),
        Operand::Str(s) => Value::Str(s.clone()),
        Operand::Bool(b) => Value::Bool(*b),
        Operand::Data(name) => data
            .get(name)
            .cloned()
            .ok_or_else(|| CondError::UnknownData(name.clone()))?,
        Operand::Visits(state) => Value::Int(visits.get(state).copied().unwrap_or(0).into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("max_rounds".to_string(), Value::Int(2)),
            ("mode".to_string(), Value::Str("fast".to_string())),
            ("strict".to_string(), Value::Bool(true)),
        ])
    }

    fn visits() -> BTreeMap<String, u32> {
        BTreeMap::from([("implement".to_string(), 1)])
    }

    fn eval(cond: &str) -> Result<bool, CondError> {
        parse(cond)
            .unwrap_or_else(|e| panic!("{cond}: {e}"))
            .eval(&data(), &visits())
    }

    // One test per operator.

    #[test]
    fn op_eq() {
        assert_eq!(eval("1 == 1"), Ok(true));
        assert_eq!(eval("1 == 2"), Ok(false));
    }

    #[test]
    fn op_ne() {
        assert_eq!(eval("1 != 2"), Ok(true));
        assert_eq!(eval("1 != 1"), Ok(false));
    }

    #[test]
    fn op_lt() {
        assert_eq!(eval("1 < 2"), Ok(true));
        assert_eq!(eval("2 < 2"), Ok(false));
    }

    #[test]
    fn op_le() {
        assert_eq!(eval("2 <= 2"), Ok(true));
        assert_eq!(eval("3 <= 2"), Ok(false));
    }

    #[test]
    fn op_gt() {
        assert_eq!(eval("3 > 2"), Ok(true));
        assert_eq!(eval("2 > 2"), Ok(false));
    }

    #[test]
    fn op_ge() {
        assert_eq!(eval("2 >= 2"), Ok(true));
        assert_eq!(eval("1 >= 2"), Ok(false));
    }

    // One test per operand kind.

    #[test]
    fn operand_int_literal() {
        assert_eq!(parse("-3 < 0").unwrap().left, Operand::Int(-3));
        assert_eq!(eval("-3 < 0"), Ok(true));
        assert_eq!(eval("10 == 010"), Ok(true));
    }

    #[test]
    fn operand_string_literal() {
        assert_eq!(
            parse(r#""a b" == "a b""#).unwrap().left,
            Operand::Str("a b".to_string())
        );
        assert_eq!(eval(r#""fast" == "fast""#), Ok(true));
        assert_eq!(eval(r#""fast" == "slow""#), Ok(false));
        assert_eq!(eval(r#""" != "x""#), Ok(true));
    }

    #[test]
    fn operand_true() {
        assert_eq!(parse("true == true").unwrap().left, Operand::Bool(true));
        assert_eq!(eval("true == true"), Ok(true));
        assert_eq!(eval("data.strict == true"), Ok(true));
    }

    #[test]
    fn operand_false() {
        assert_eq!(parse("false != true").unwrap().left, Operand::Bool(false));
        assert_eq!(eval("false != true"), Ok(true));
        assert_eq!(eval("data.strict == false"), Ok(false));
    }

    #[test]
    fn operand_data() {
        assert_eq!(
            parse("data.max_rounds == 2").unwrap().left,
            Operand::Data("max_rounds".to_string())
        );
        assert_eq!(eval("data.max_rounds == 2"), Ok(true));
        assert_eq!(eval(r#"data.mode == "fast""#), Ok(true));
        assert_eq!(eval("data.max_rounds > 2"), Ok(false));
    }

    #[test]
    fn operand_visits() {
        assert_eq!(
            parse("visits.implement < 2").unwrap().left,
            Operand::Visits("implement".to_string())
        );
        // The section 5 example: one visit to implement, max_rounds 2.
        assert_eq!(eval("visits.implement < data.max_rounds"), Ok(true));
        // A state never entered has 0 visits.
        assert_eq!(eval("visits.review == 0"), Ok(true));
        let two = BTreeMap::from([("implement".to_string(), 2)]);
        let cond = parse("visits.implement < data.max_rounds").unwrap();
        assert_eq!(cond.eval(&data(), &two), Ok(false));
    }

    // Parse failures named in M1.2.

    #[test]
    fn rejects_listed_inputs() {
        for input in [
            "a && b",
            "visits.x <",
            "data.y == 1 == 2",
            "foo == 1",
            "exit_code == 0",
            "",
        ] {
            assert!(parse(input).is_err(), "{input:?} parsed");
        }
    }

    #[test]
    fn parse_errors_say_why() {
        assert_eq!(parse(""), Err(CondError::Empty));
        assert_eq!(parse("   "), Err(CondError::Empty));
        assert_eq!(parse("a && b"), Err(CondError::UnexpectedChar('&')));
        assert_eq!(
            parse("visits.x <"),
            Err(CondError::Shape("visits.x <".to_string()))
        );
        assert_eq!(
            parse("data.y == 1 == 2"),
            Err(CondError::Shape("data.y == 1 == 2".to_string()))
        );
        assert_eq!(
            parse("foo == 1"),
            Err(CondError::UnknownOperand("foo".to_string()))
        );
        assert_eq!(
            parse("exit_code == 0"),
            Err(CondError::UnknownOperand("exit_code".to_string()))
        );
    }

    #[test]
    fn rejects_other_shapes() {
        for input in [
            "1 || 2",
            "(1 == 1)",
            "!true == false",
            "1 = 1",
            "1 === 1",
            "1 <> 2",
            "data. == 1",
            "data.Name == 1",
            "data.a.b == 1",
            "visits. == 1",
            "1 == \"open",
            r#""a\"b" == "a""#,
            "1.5 == 1",
            "99999999999999999999 > 0",
            "True == true",
            "1 2",
            "== ==",
        ] {
            assert!(parse(input).is_err(), "{input:?} parsed");
        }
    }

    #[test]
    fn whitespace_is_optional() {
        assert_eq!(
            parse("visits.implement<data.max_rounds"),
            parse("  visits.implement   <   data.max_rounds ")
        );
    }

    #[test]
    fn display_round_trips() {
        for input in [
            "visits.implement < data.max_rounds",
            r#"data.mode != "slow""#,
            "true == false",
            "-1 >= 0",
        ] {
            let cond = parse(input).unwrap();
            assert_eq!(cond.to_string(), input);
            assert_eq!(cond.to_string().parse::<Cond>(), Ok(cond));
        }
    }

    #[test]
    fn refs_for_validation() {
        let cond = parse("visits.implement < data.max_rounds").unwrap();
        assert_eq!(cond.data_refs().collect::<Vec<_>>(), ["max_rounds"]);
        assert_eq!(cond.visits_refs().collect::<Vec<_>>(), ["implement"]);
    }

    #[test]
    fn eval_errors() {
        assert_eq!(
            eval("data.missing == 1"),
            Err(CondError::UnknownData("missing".to_string()))
        );
        assert_eq!(
            eval(r#"data.mode == 1"#),
            Err(CondError::TypeMismatch {
                left: "a string",
                right: "an int"
            })
        );
        assert_eq!(
            eval("true == 1"),
            Err(CondError::TypeMismatch {
                left: "a bool",
                right: "an int"
            })
        );
        assert_eq!(
            eval(r#""a" < "b""#),
            Err(CondError::NotOrdered {
                op: Op::Lt,
                kind: "a string"
            })
        );
        assert_eq!(
            eval("false >= true"),
            Err(CondError::NotOrdered {
                op: Op::Ge,
                kind: "a bool"
            })
        );
    }
}
