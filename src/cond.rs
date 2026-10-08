//! Conditions of `check` invokes (docs/reference/machines.md, Conditions): typed objects with
//! exactly one subject and one operator, so the YAML parser and `decree check` catch mistakes.
//!
//! ```yaml
//! { output: read_text, matches: '^ok' }
//! { data: file, matches: '\.md$' }
//! { visits: implement, less_than: { data: max_rounds } }
//! { data: mode, equals: fast }
//! { confidence: big_model, at_least: 0.4 }
//! ```
//!
//! The reference defines no type coercion, so evaluation refuses to compare values of different
//! types, except an int with a `number`, and the ordering operators apply to ints and numbers
//! only. Floats appear in `confidence`, a number from 0 to 1, and in `number` data.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A `check` condition, as written. `shape` checks that it has exactly one subject and one
/// operator that subject takes (V10).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equals: Option<Operand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_equals: Option<Operand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub less_than: Option<Operand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_most: Option<Operand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_than: Option<Operand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_least: Option<Operand>,
}

/// The value an operator compares with: a literal, or `{ data: <name> }`. Floats are for
/// `confidence` only (V10).
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Data(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Equals,
    NotEquals,
    LessThan,
    AtMost,
    MoreThan,
    AtLeast,
}

/// A condition's subject and its test.
type Shape<'a> = (Subject<'a>, Test<'a>);

/// What a condition tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject<'a> {
    /// The latest script output of this state, as logged.
    Output(&'a str),
    /// How many times this state has been entered.
    Visits(&'a str),
    /// This `data` value.
    Data(&'a str),
    /// The confidence of this state's latest `model` decision.
    Confidence(&'a str),
}

/// What a subject is tested with: a comparison, or (on `output` and `data`) a regular
/// expression.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Test<'a> {
    Compare(Op, &'a Operand),
    Matches(&'a str),
}

/// What a condition is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    /// The run's `data`.
    pub data: &'a BTreeMap<String, Value>,
    /// Visit counts; a state missing here has not been entered.
    pub visits: &'a BTreeMap<String, u32>,
    /// The confidence of each state's latest decision; a state missing here counts 0.
    pub confidence: &'a BTreeMap<String, f64>,
    /// The logged output of the state an `output` subject names; empty if it has not run.
    pub output: &'a str,
}

/// A value a condition compares: a literal, a `data` value or a visit count.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    /// A `number` data value, or a float literal it is compared with.
    Number(f64),
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CondError {
    #[error("a condition needs one subject: `output`, `visits`, `data` or `confidence`")]
    NoSubject,
    #[error("a condition has exactly one subject, not {0}; use two `check` states in a row")]
    ManySubjects(String),
    #[error("`{}` needs one operator: {}", .0, operators_of(.0))]
    NoOperator(&'static str),
    #[error("a condition has exactly one operator, not {0}; use two `check` states in a row")]
    ManyOperators(String),
    #[error("`output` takes the operator `matches`, not `{0}`")]
    CompareOutput(&'static str),
    #[error("`matches` tests `output` or a `data` value, not `{0}`")]
    MatchesOn(&'static str),
    #[error("`matches` needs a string, not {0}")]
    MatchesNotString(&'static str),
    #[error("`matches` '{pattern}' is not a regular expression: {message}")]
    Regex { pattern: String, message: String },
    #[error("unknown data `{0}`")]
    UnknownData(String),
    #[error("cannot compare {left} with {right}")]
    TypeMismatch {
        left: &'static str,
        right: &'static str,
    },
    #[error("`{op}` compares numbers only, not {kind}")]
    NotOrdered { op: Op, kind: &'static str },
    #[error("`confidence` compares to a number from 0 to 1, not {0}")]
    NotConfidence(String),
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Op::Equals => "equals",
            Op::NotEquals => "not_equals",
            Op::LessThan => "less_than",
            Op::AtMost => "at_most",
            Op::MoreThan => "more_than",
            Op::AtLeast => "at_least",
        }
    }

    /// Compare two ints or two numbers.
    fn compare<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Op::Equals => a == b,
            Op::NotEquals => a != b,
            Op::LessThan => a < b,
            Op::AtMost => a <= b,
            Op::MoreThan => a > b,
            Op::AtLeast => a >= b,
        }
    }

    /// Whether the operator orders its operands, and so compares ints and numbers only.
    pub fn is_ordering(self) -> bool {
        !matches!(self, Op::Equals | Op::NotEquals)
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "an int",
            Value::Number(_) => "a number",
            Value::Str(_) => "a string",
            Value::Bool(_) => "a bool",
        }
    }
}

impl fmt::Display for Operand {
    /// As a graph note shows it: `3`, `fast`, `true` or `data.max_rounds`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Int(n) => write!(f, "{n}"),
            Operand::Float(x) => write!(f, "{x}"),
            Operand::Str(s) => f.write_str(s),
            Operand::Bool(b) => write!(f, "{b}"),
            Operand::Data(name) => write!(f, "data.{name}"),
        }
    }
}

impl<'de> Deserialize<'de> for Operand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde_norway::Value as Yaml;
        let expected = "a value is a number, a string, a bool or `{ data: <name> }`";
        match Yaml::deserialize(deserializer)? {
            Yaml::String(s) => Ok(Operand::Str(s)),
            Yaml::Bool(b) => Ok(Operand::Bool(b)),
            Yaml::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => Ok(Operand::Int(i)),
                (None, Some(x)) => Ok(Operand::Float(x)),
                (None, None) => Err(D::Error::custom(format!("`{n}`: {expected}"))),
            },
            Yaml::Mapping(map) => {
                let mut entries = map.into_iter();
                match (entries.next(), entries.next()) {
                    (Some((Yaml::String(key), Yaml::String(name))), None) if key == "data" => {
                        Ok(Operand::Data(name))
                    }
                    _ => Err(D::Error::custom(expected)),
                }
            }
            _ => Err(D::Error::custom(expected)),
        }
    }
}

impl Serialize for Operand {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Operand::Int(n) => serializer.serialize_i64(*n),
            Operand::Float(x) => serializer.serialize_f64(*x),
            Operand::Str(s) => serializer.serialize_str(s),
            Operand::Bool(b) => serializer.serialize_bool(*b),
            Operand::Data(name) => BTreeMap::from([("data", name)]).serialize(serializer),
        }
    }
}

impl Condition {
    /// Every operator that is set, in declaration order.
    fn operators(&self) -> Vec<(Op, &Operand)> {
        [
            (Op::Equals, &self.equals),
            (Op::NotEquals, &self.not_equals),
            (Op::LessThan, &self.less_than),
            (Op::AtMost, &self.at_most),
            (Op::MoreThan, &self.more_than),
            (Op::AtLeast, &self.at_least),
        ]
        .into_iter()
        .filter_map(|(op, value)| value.as_ref().map(|v| (op, v)))
        .collect()
    }

    /// The subject and the test: exactly one of each (V10).
    pub fn shape(&self) -> Result<Shape<'_>, CondError> {
        let subjects: Vec<Subject> = [
            self.output.as_deref().map(Subject::Output),
            self.visits.as_deref().map(Subject::Visits),
            self.data.as_deref().map(Subject::Data),
            self.confidence.as_deref().map(Subject::Confidence),
        ]
        .into_iter()
        .flatten()
        .collect();
        let subject = match subjects.as_slice() {
            [] => return Err(CondError::NoSubject),
            [one] => *one,
            many => {
                let names: Vec<String> = many.iter().map(|s| format!("`{}`", s.key())).collect();
                return Err(CondError::ManySubjects(names.join(" and ")));
            }
        };
        let mut tests: Vec<Test> = self
            .matches
            .as_deref()
            .map(Test::Matches)
            .into_iter()
            .collect();
        tests.extend(
            self.operators()
                .into_iter()
                .map(|(op, v)| Test::Compare(op, v)),
        );
        match (subject, tests.as_slice()) {
            (_, []) => Err(CondError::NoOperator(subject.key())),
            (Subject::Output(_), [Test::Compare(op, _)]) => {
                Err(CondError::CompareOutput(op.as_str()))
            }
            (Subject::Visits(_) | Subject::Confidence(_), [Test::Matches(_)]) => {
                Err(CondError::MatchesOn(subject.key()))
            }
            (_, [one]) => Ok((subject, *one)),
            (_, many) => {
                let names: Vec<String> = many.iter().map(|t| format!("`{}`", t.key())).collect();
                Err(CondError::ManyOperators(names.join(" and ")))
            }
        }
    }

    /// Evaluate against the run's facts: `data`, visit counts, decision confidences and the
    /// output of the state an `output` subject names.
    pub fn eval(&self, facts: &Facts) -> Result<bool, CondError> {
        let (op, left, operand) = match self.shape()? {
            (Subject::Output(_), Test::Matches(pattern)) => {
                return Ok(compile(pattern)?.is_match(facts.output))
            }
            (Subject::Data(name), Test::Matches(pattern)) => {
                return match lookup(facts.data, name)? {
                    Value::Str(s) => Ok(compile(pattern)?.is_match(&s)),
                    other => Err(CondError::MatchesNotString(other.kind())),
                };
            }
            (Subject::Confidence(state), Test::Compare(op, operand)) => {
                let left = facts.confidence.get(state).copied().unwrap_or(0.0);
                return Ok(op.compare(left, confidence_operand(operand)?));
            }
            (Subject::Visits(state), Test::Compare(op, operand)) => {
                let visits = facts.visits.get(state).copied().unwrap_or(0);
                (op, Value::Int(visits.into()), operand)
            }
            (Subject::Data(name), Test::Compare(op, operand)) => {
                (op, lookup(facts.data, name)?, operand)
            }
            // `shape` gives `output` only `matches`, and `visits` and `confidence` only a
            // comparison.
            (subject, _) => return Err(CondError::MatchesOn(subject.key())),
        };
        let right = match operand {
            Operand::Int(n) => Value::Int(*n),
            Operand::Float(x) if matches!(left, Value::Number(_)) => Value::Number(*x),
            Operand::Float(_) => {
                return Err(CondError::TypeMismatch {
                    left: left.kind(),
                    right: "a float",
                });
            }
            Operand::Str(s) => Value::Str(s.clone()),
            Operand::Bool(b) => Value::Bool(*b),
            Operand::Data(name) => lookup(facts.data, name)?,
        };
        compare(&left, op, &right)
    }
}

/// The number a `confidence` compares to: an int or a float from 0 to 1, not `data`.
pub fn confidence_operand(operand: &Operand) -> Result<f64, CondError> {
    let x = match operand {
        Operand::Int(n) => *n as f64,
        Operand::Float(x) => *x,
        other => return Err(CondError::NotConfidence(other.to_string())),
    };
    if (0.0..=1.0).contains(&x) {
        Ok(x)
    } else {
        Err(CondError::NotConfidence(operand.to_string()))
    }
}

impl Test<'_> {
    fn key(self) -> &'static str {
        match self {
            Test::Compare(op, _) => op.as_str(),
            Test::Matches(_) => "matches",
        }
    }
}

impl Subject<'_> {
    pub fn key(self) -> &'static str {
        match self {
            Subject::Output(_) => "output",
            Subject::Visits(_) => "visits",
            Subject::Data(_) => "data",
            Subject::Confidence(_) => "confidence",
        }
    }
}

/// The operators the subject `key` takes, for [`CondError::NoOperator`].
fn operators_of(key: &str) -> &'static str {
    match key {
        "output" => "`matches`",
        "data" => {
            "`matches`, `equals`, `not_equals`, `less_than`, `at_most`, `more_than` or `at_least`"
        }
        _ => "`equals`, `not_equals`, `less_than`, `at_most`, `more_than` or `at_least`",
    }
}

impl fmt::Display for Condition {
    /// As a graph note shows it: `<subject> <name> <op> <value>`, e.g.
    /// `visits implement less_than data.max_rounds`, `data file matches '<regex>'`,
    /// `confidence big_model at_least 0.4`, or `output read_text matches '<regex>'`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut words = Vec::new();
        for (key, name) in [
            ("output", &self.output),
            ("visits", &self.visits),
            ("data", &self.data),
            ("confidence", &self.confidence),
        ] {
            if let Some(name) = name {
                words.push(format!("{key} {name}"));
            }
        }
        if let Some(pattern) = &self.matches {
            words.push(format!("matches '{pattern}'"));
        }
        for (op, value) in self.operators() {
            words.push(format!("{op} {value}"));
        }
        f.write_str(&words.join(" "))
    }
}

/// Compile a `matches` pattern.
pub fn compile(pattern: &str) -> Result<regex::Regex, CondError> {
    regex::Regex::new(pattern).map_err(|e| CondError::Regex {
        pattern: pattern.to_string(),
        message: e
            .to_string()
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .trim_start_matches("error: ")
            .to_string(),
    })
}

fn lookup(data: &BTreeMap<String, Value>, name: &str) -> Result<Value, CondError> {
    data.get(name)
        .cloned()
        .ok_or_else(|| CondError::UnknownData(name.to_string()))
}

fn compare(left: &Value, op: Op, right: &Value) -> Result<bool, CondError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Ok(op.compare(a, b)),
        (Value::Number(a), Value::Number(b)) => Ok(op.compare(a, b)),
        (Value::Number(a), Value::Int(b)) => Ok(op.compare(*a, *b as f64)),
        (Value::Int(a), Value::Number(b)) => Ok(op.compare(*a as f64, *b)),
        (Value::Str(_), Value::Str(_)) | (Value::Bool(_), Value::Bool(_)) => match op {
            Op::Equals => Ok(left == right),
            Op::NotEquals => Ok(left != right),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Condition {
        serde_norway::from_str(yaml).unwrap_or_else(|e| panic!("{yaml}: {e}"))
    }

    fn data() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("max_rounds".to_string(), Value::Int(2)),
            ("mode".to_string(), Value::Str("fast".to_string())),
            ("strict".to_string(), Value::Bool(true)),
            ("file".to_string(), Value::Str("notes/a.md".to_string())),
            ("megapixels".to_string(), Value::Number(0.5)),
        ])
    }

    fn visits() -> BTreeMap<String, u32> {
        BTreeMap::from([("implement".to_string(), 1)])
    }

    fn confidence() -> BTreeMap<String, f64> {
        BTreeMap::from([("big_model".to_string(), 0.55)])
    }

    fn eval_with(
        yaml: &str,
        data: &BTreeMap<String, Value>,
        visits: &BTreeMap<String, u32>,
        confidence: &BTreeMap<String, f64>,
    ) -> Result<bool, CondError> {
        parse(yaml).eval(&Facts {
            data,
            visits,
            confidence,
            output: "tests: 3 passed\n[stderr] warning\n",
        })
    }

    fn eval(yaml: &str) -> Result<bool, CondError> {
        eval_with(yaml, &data(), &visits(), &confidence())
    }

    // One test per operator, on each subject that takes one.

    #[test]
    fn op_equals() {
        assert_eq!(eval("{ visits: implement, equals: 1 }"), Ok(true));
        assert_eq!(eval("{ data: mode, equals: fast }"), Ok(true));
        assert_eq!(eval("{ data: strict, equals: false }"), Ok(false));
    }

    #[test]
    fn op_not_equals() {
        assert_eq!(eval("{ visits: implement, not_equals: 1 }"), Ok(false));
        assert_eq!(eval("{ data: mode, not_equals: slow }"), Ok(true));
        assert_eq!(eval("{ data: strict, not_equals: false }"), Ok(true));
    }

    #[test]
    fn op_less_than() {
        assert_eq!(eval("{ visits: implement, less_than: 2 }"), Ok(true));
        assert_eq!(eval("{ visits: implement, less_than: 1 }"), Ok(false));
    }

    #[test]
    fn op_at_most() {
        assert_eq!(eval("{ visits: implement, at_most: 1 }"), Ok(true));
        assert_eq!(eval("{ visits: implement, at_most: 0 }"), Ok(false));
    }

    #[test]
    fn op_more_than() {
        assert_eq!(eval("{ data: max_rounds, more_than: 1 }"), Ok(true));
        assert_eq!(eval("{ data: max_rounds, more_than: 2 }"), Ok(false));
    }

    #[test]
    fn op_at_least() {
        assert_eq!(eval("{ data: max_rounds, at_least: 2 }"), Ok(true));
        assert_eq!(eval("{ data: max_rounds, at_least: 3 }"), Ok(false));
    }

    #[test]
    fn number_data_compares_numerically() {
        assert_eq!(eval("{ data: megapixels, less_than: 0.75 }"), Ok(true));
        assert_eq!(eval("{ data: megapixels, at_least: 1 }"), Ok(false));
        assert_eq!(eval("{ data: megapixels, equals: 0.5 }"), Ok(true));
        assert_eq!(
            eval("{ data: megapixels, less_than: { data: max_rounds } }"),
            Ok(true)
        );
        assert_eq!(
            eval("{ data: max_rounds, more_than: { data: megapixels } }"),
            Ok(true)
        );
        assert_eq!(
            eval("{ data: megapixels, equals: fast }"),
            Err(CondError::TypeMismatch {
                left: "a number",
                right: "a string"
            })
        );
        assert_eq!(
            eval("{ data: max_rounds, less_than: 2.5 }"),
            Err(CondError::TypeMismatch {
                left: "an int",
                right: "a float"
            })
        );
    }

    // One test per subject.

    #[test]
    fn subject_output_reads_the_state_output_as_logged() {
        assert_eq!(eval(r"{ output: s, matches: '\d+ passed' }"), Ok(true));
        assert_eq!(
            eval("{ output: s, matches: '^\\[stderr\\] warning$' }"),
            Ok(false)
        );
        assert_eq!(
            eval("{ output: s, matches: '(?m)^\\[stderr\\] warning$' }"),
            Ok(true)
        );
        assert_eq!(eval("{ output: s, matches: failed }"), Ok(false));
    }

    #[test]
    fn subject_visits_counts_zero_for_a_state_never_entered() {
        assert_eq!(eval("{ visits: review, equals: 0 }"), Ok(true));
    }

    #[test]
    fn subject_data_with_matches_tests_the_string_value() {
        assert_eq!(eval(r"{ data: file, matches: '\.md$' }"), Ok(true));
        let txt = BTreeMap::from([("file".to_string(), Value::Str("notes/a.txt".to_string()))]);
        assert_eq!(
            eval_with(
                r"{ data: file, matches: '\.md$' }",
                &txt,
                &visits(),
                &confidence()
            ),
            Ok(false)
        );
        // It reads the value, not the output.
        assert_eq!(eval("{ data: file, matches: passed }"), Ok(false));
    }

    #[test]
    fn subject_confidence_reads_the_latest_decision() {
        assert_eq!(eval("{ confidence: big_model, at_least: 0.4 }"), Ok(true));
        assert_eq!(eval("{ confidence: big_model, at_least: 0.6 }"), Ok(false));
        assert_eq!(eval("{ confidence: big_model, less_than: 1 }"), Ok(true));
        assert_eq!(eval("{ confidence: big_model, equals: 0.55 }"), Ok(true));
        assert_eq!(
            eval("{ confidence: big_model, not_equals: 0.55 }"),
            Ok(false)
        );
        assert_eq!(eval("{ confidence: big_model, at_most: 0.55 }"), Ok(true));
        assert_eq!(
            eval("{ confidence: big_model, more_than: 0.55 }"),
            Ok(false)
        );
    }

    #[test]
    fn subject_confidence_is_zero_when_none_was_reported() {
        assert_eq!(
            eval("{ confidence: local_model, at_least: 0.4 }"),
            Ok(false)
        );
        assert_eq!(eval("{ confidence: local_model, equals: 0 }"), Ok(true));
    }

    #[test]
    fn subject_data() {
        assert_eq!(eval("{ data: strict, equals: true }"), Ok(true));
    }

    #[test]
    fn data_value_on_the_right() {
        // The docs/reference/machines.md example: one visit to implement, max_rounds 2.
        let cond = parse("{ visits: implement, less_than: { data: max_rounds } }");
        assert_eq!(
            cond.less_than,
            Some(Operand::Data("max_rounds".to_string()))
        );
        assert_eq!(
            eval_with(
                "{ visits: implement, less_than: { data: max_rounds } }",
                &data(),
                &visits(),
                &confidence()
            ),
            Ok(true)
        );
        let two = BTreeMap::from([("implement".to_string(), 2)]);
        assert_eq!(
            eval_with(
                "{ visits: implement, less_than: { data: max_rounds } }",
                &data(),
                &two,
                &confidence()
            ),
            Ok(false)
        );
    }

    #[test]
    fn yes_no_and_on_are_strings() {
        // YAML 1.2: the bare words YAML 1.1 reads as booleans stay strings.
        let cond = parse("{ data: mode, equals: no }");
        assert_eq!(cond.equals, Some(Operand::Str("no".to_string())));
    }

    // Shape (V10).

    #[test]
    fn shape_needs_one_subject_and_one_operator() {
        assert_eq!(parse("{ equals: 1 }").shape(), Err(CondError::NoSubject));
        assert_eq!(
            parse("{ visits: a, data: b, equals: 1 }").shape(),
            Err(CondError::ManySubjects("`visits` and `data`".into()))
        );
        assert_eq!(
            parse("{ visits: a }").shape(),
            Err(CondError::NoOperator("visits"))
        );
        assert_eq!(
            parse("{ data: a, equals: 1, less_than: 2 }").shape(),
            Err(CondError::ManyOperators("`equals` and `less_than`".into()))
        );
        assert_eq!(parse("{ matches: x }").shape(), Err(CondError::NoSubject));
        assert_eq!(
            parse("{ output: s, equals: 1 }").shape(),
            Err(CondError::CompareOutput("equals"))
        );
        assert_eq!(
            parse("{ output: s }").shape(),
            Err(CondError::NoOperator("output"))
        );
        assert_eq!(
            parse("{ output: s, matches: x }").shape(),
            Ok((Subject::Output("s"), Test::Matches("x")))
        );
        assert_eq!(
            parse("{ data: f, matches: x }").shape(),
            Ok((Subject::Data("f"), Test::Matches("x")))
        );
        assert_eq!(
            parse("{ data: f, matches: x, equals: y }").shape(),
            Err(CondError::ManyOperators("`matches` and `equals`".into()))
        );
        assert_eq!(
            parse("{ output: s, data: f, matches: x }").shape(),
            Err(CondError::ManySubjects("`output` and `data`".into()))
        );
        assert_eq!(
            parse("{ visits: a, matches: x }").shape(),
            Err(CondError::MatchesOn("visits"))
        );
        assert_eq!(
            parse("{ confidence: a, matches: x }").shape(),
            Err(CondError::MatchesOn("confidence"))
        );
        assert_eq!(
            parse("{ confidence: a }").shape(),
            Err(CondError::NoOperator("confidence"))
        );
        assert_eq!(
            parse("{ confidence: a, visits: b, at_least: 0.5 }").shape(),
            Err(CondError::ManySubjects("`visits` and `confidence`".into()))
        );
    }

    #[test]
    fn unknown_keys_and_bad_values_fail_to_parse() {
        for yaml in [
            "{ visits: a, lt: 1 }",
            "{ and: [] }",
            "{ visits: a, equals: { state: b } }",
            "{ visits: a, equals: { data: b, other: c } }",
            "{ visits: a, equals: [1] }",
        ] {
            assert!(
                serde_norway::from_str::<Condition>(yaml).is_err(),
                "{yaml} parsed"
            );
        }
    }

    #[test]
    fn numbers_parse_as_ints_or_floats() {
        assert_eq!(
            parse("{ visits: a, equals: 1 }").equals,
            Some(Operand::Int(1))
        );
        assert_eq!(
            parse("{ confidence: a, at_least: 0.25 }").at_least,
            Some(Operand::Float(0.25))
        );
    }

    #[test]
    fn confidence_operand_is_a_number_from_0_to_1() {
        assert_eq!(confidence_operand(&Operand::Float(0.25)), Ok(0.25));
        assert_eq!(confidence_operand(&Operand::Int(1)), Ok(1.0));
        assert_eq!(
            confidence_operand(&Operand::Float(1.5)),
            Err(CondError::NotConfidence("1.5".into()))
        );
        assert_eq!(
            confidence_operand(&Operand::Int(-1)),
            Err(CondError::NotConfidence("-1".into()))
        );
        assert_eq!(
            confidence_operand(&Operand::Data("x".into())),
            Err(CondError::NotConfidence("data.x".into()))
        );
        assert_eq!(
            confidence_operand(&Operand::Str("high".into())),
            Err(CondError::NotConfidence("high".into()))
        );
    }

    #[test]
    fn eval_errors() {
        assert_eq!(
            eval("{ data: missing, equals: 1 }"),
            Err(CondError::UnknownData("missing".to_string()))
        );
        assert_eq!(
            eval("{ data: mode, equals: 1 }"),
            Err(CondError::TypeMismatch {
                left: "a string",
                right: "an int"
            })
        );
        assert_eq!(
            eval("{ data: mode, less_than: b }"),
            Err(CondError::NotOrdered {
                op: Op::LessThan,
                kind: "a string"
            })
        );
        assert!(matches!(
            eval("{ output: s, matches: '(' }"),
            Err(CondError::Regex { .. })
        ));
        assert_eq!(
            eval("{ data: max_rounds, matches: '2' }"),
            Err(CondError::MatchesNotString("an int"))
        );
        assert_eq!(
            eval("{ confidence: big_model, at_least: 1.5 }"),
            Err(CondError::NotConfidence("1.5".into()))
        );
        assert_eq!(
            eval("{ visits: implement, equals: 1.5 }"),
            Err(CondError::TypeMismatch {
                left: "an int",
                right: "a float"
            })
        );
    }

    #[test]
    fn display_is_the_graph_note_form() {
        assert_eq!(
            parse("{ visits: implement, less_than: { data: max_rounds } }").to_string(),
            "visits implement less_than data.max_rounds"
        );
        assert_eq!(
            parse("{ output: read_text, matches: '^ok$' }").to_string(),
            "output read_text matches '^ok$'"
        );
        assert_eq!(
            parse("{ data: strict, equals: true }").to_string(),
            "data strict equals true"
        );
        assert_eq!(
            parse(r"{ data: file, matches: '\.md$' }").to_string(),
            r"data file matches '\.md$'"
        );
        assert_eq!(
            parse("{ confidence: big_model, at_least: 0.4 }").to_string(),
            "confidence big_model at_least 0.4"
        );
    }

    #[test]
    fn serializes_as_written() {
        let cond = parse("{ visits: implement, less_than: { data: max_rounds } }");
        assert_eq!(
            serde_json::to_value(&cond).unwrap(),
            serde_json::json!({ "visits": "implement", "less_than": { "data": "max_rounds" } })
        );
        let cond = parse("{ confidence: big_model, at_least: 0.4 }");
        assert_eq!(
            serde_json::to_string(&cond).unwrap(),
            r#"{"confidence":"big_model","at_least":0.4}"#
        );
    }
}
