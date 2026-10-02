//! Conditions of `check` invokes (spec section 5, Conditions): typed objects with one
//! subject and, for `visits` and `data`, one operator, so the YAML parser and `decree check`
//! catch mistakes.
//!
//! ```yaml
//! { matches: '^ok' }
//! { visits: implement, less_than: { data: max_rounds } }
//! { data: mode, equals: fast }
//! ```
//!
//! The spec defines no type coercion, so evaluation refuses to compare values of different
//! types, and the ordering operators apply to integers only.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A `check` condition, as written. `shape` checks that it has exactly one subject and the
/// operators that subject needs (V10).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
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

/// The value an operator compares with: a literal, or `{ data: <name> }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Int(i64),
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

/// A condition's subject and, for `visits` and `data`, its operator and value.
pub type Shape<'a> = (Subject<'a>, Option<(Op, &'a Operand)>);

/// What a condition tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject<'a> {
    /// The input matches this regular expression.
    Matches(&'a str),
    /// How many times this state has been entered.
    Visits(&'a str),
    /// This `data` value.
    Data(&'a str),
}

/// A value a condition compares: a literal, a `data` value or a visit count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CondError {
    #[error("a condition needs one subject: `matches`, `visits` or `data`")]
    NoSubject,
    #[error("a condition has exactly one subject, not {0}; use two `check` states in a row")]
    ManySubjects(String),
    #[error("`{0}` needs one operator: `equals`, `not_equals`, `less_than`, `at_most`, `more_than` or `at_least`")]
    NoOperator(&'static str),
    #[error("a condition has exactly one operator, not {0}; use two `check` states in a row")]
    ManyOperators(String),
    #[error("`matches` takes no operator, but has `{0}`")]
    OperatorOnMatches(&'static str),
    #[error("`matches` '{pattern}' is not a regular expression: {message}")]
    Regex { pattern: String, message: String },
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

impl Op {
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Equals => "equals",
            Op::NotEquals => "not_equals",
            Op::LessThan => "less_than",
            Op::AtMost => "at_most",
            Op::MoreThan => "more_than",
            Op::AtLeast => "at_least",
        }
    }

    /// Whether the operator orders its operands, and so compares integers only.
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
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Int(_) => "an int",
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
            Operand::Str(s) => f.write_str(s),
            Operand::Bool(b) => write!(f, "{b}"),
            Operand::Data(name) => write!(f, "data.{name}"),
        }
    }
}

impl<'de> Deserialize<'de> for Operand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde_norway::Value as Yaml;
        let expected = "a value is an int, a string, a bool or `{ data: <name> }`";
        match Yaml::deserialize(deserializer)? {
            Yaml::String(s) => Ok(Operand::Str(s)),
            Yaml::Bool(b) => Ok(Operand::Bool(b)),
            Yaml::Number(n) => n
                .as_i64()
                .map(Operand::Int)
                .ok_or_else(|| D::Error::custom(format!("`{n}`: {expected}"))),
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
            Operand::Str(s) => serializer.serialize_str(s),
            Operand::Bool(b) => serializer.serialize_bool(*b),
            Operand::Data(name) => BTreeMap::from([("data", name)]).serialize(serializer),
        }
    }
}

impl Condition {
    /// Every operator that is set, in declaration order.
    pub fn operators(&self) -> Vec<(Op, &Operand)> {
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

    /// The subject and, for `visits` and `data`, the operator: exactly one of each (V10).
    pub fn shape(&self) -> Result<Shape<'_>, CondError> {
        let subjects: Vec<Subject> = [
            self.matches.as_deref().map(Subject::Matches),
            self.visits.as_deref().map(Subject::Visits),
            self.data.as_deref().map(Subject::Data),
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
        let ops = self.operators();
        match (subject, ops.as_slice()) {
            (Subject::Matches(_), []) => Ok((subject, None)),
            (Subject::Matches(_), [(op, _), ..]) => Err(CondError::OperatorOnMatches(op.as_str())),
            (_, []) => Err(CondError::NoOperator(subject.key())),
            (_, [one]) => Ok((subject, Some(*one))),
            (_, many) => {
                let names: Vec<String> = many.iter().map(|(op, _)| format!("`{op}`")).collect();
                Err(CondError::ManyOperators(names.join(" and ")))
            }
        }
    }

    /// Evaluate against the run's `data`, its visit counts and the input text. A state
    /// missing from `visits` has not been entered, so its count is 0.
    pub fn eval(
        &self,
        data: &BTreeMap<String, Value>,
        visits: &BTreeMap<String, u32>,
        input: &str,
    ) -> Result<bool, CondError> {
        let (subject, op) = self.shape()?;
        let left = match subject {
            Subject::Matches(pattern) => return Ok(compile(pattern)?.is_match(input)),
            Subject::Visits(state) => Value::Int(visits.get(state).copied().unwrap_or(0).into()),
            Subject::Data(name) => lookup(data, name)?,
        };
        let Some((op, operand)) = op else {
            unreachable!("shape gives `visits` and `data` an operator");
        };
        let right = match operand {
            Operand::Int(n) => Value::Int(*n),
            Operand::Str(s) => Value::Str(s.clone()),
            Operand::Bool(b) => Value::Bool(*b),
            Operand::Data(name) => lookup(data, name)?,
        };
        compare(&left, op, &right)
    }
}

impl Subject<'_> {
    pub fn key(self) -> &'static str {
        match self {
            Subject::Matches(_) => "matches",
            Subject::Visits(_) => "visits",
            Subject::Data(_) => "data",
        }
    }
}

impl fmt::Display for Condition {
    /// As a graph note shows it: `<subject> <name> <op> <value>`, e.g.
    /// `visits implement less_than data.max_rounds`, or `matches '<regex>'`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut words = Vec::new();
        if let Some(pattern) = &self.matches {
            words.push(format!("matches '{pattern}'"));
        }
        if let Some(state) = &self.visits {
            words.push(format!("visits {state}"));
        }
        if let Some(name) = &self.data {
            words.push(format!("data {name}"));
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
        (Value::Int(a), Value::Int(b)) => Ok(match op {
            Op::Equals => a == b,
            Op::NotEquals => a != b,
            Op::LessThan => a < b,
            Op::AtMost => a <= b,
            Op::MoreThan => a > b,
            Op::AtLeast => a >= b,
        }),
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
        ])
    }

    fn visits() -> BTreeMap<String, u32> {
        BTreeMap::from([("implement".to_string(), 1)])
    }

    fn eval(yaml: &str) -> Result<bool, CondError> {
        parse(yaml).eval(&data(), &visits(), "tests: 3 passed\n[stderr] warning\n")
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

    // One test per subject.

    #[test]
    fn subject_matches_reads_the_input_as_logged() {
        assert_eq!(eval(r"{ matches: '\d+ passed' }"), Ok(true));
        assert_eq!(eval("{ matches: '^\\[stderr\\] warning$' }"), Ok(false));
        assert_eq!(eval("{ matches: '(?m)^\\[stderr\\] warning$' }"), Ok(true));
        assert_eq!(eval("{ matches: failed }"), Ok(false));
    }

    #[test]
    fn subject_visits_counts_zero_for_a_state_never_entered() {
        assert_eq!(eval("{ visits: review, equals: 0 }"), Ok(true));
    }

    #[test]
    fn subject_data() {
        assert_eq!(eval("{ data: strict, equals: true }"), Ok(true));
    }

    #[test]
    fn data_value_on_the_right() {
        // The section 5 example: one visit to implement, max_rounds 2.
        let cond = parse("{ visits: implement, less_than: { data: max_rounds } }");
        assert_eq!(
            cond.less_than,
            Some(Operand::Data("max_rounds".to_string()))
        );
        assert_eq!(cond.eval(&data(), &visits(), ""), Ok(true));
        let two = BTreeMap::from([("implement".to_string(), 2)]);
        assert_eq!(cond.eval(&data(), &two, ""), Ok(false));
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
        assert_eq!(
            parse("{ matches: x, equals: 1 }").shape(),
            Err(CondError::OperatorOnMatches("equals"))
        );
        assert!(parse("{ matches: x }").shape().is_ok());
    }

    #[test]
    fn unknown_keys_and_bad_values_fail_to_parse() {
        for yaml in [
            "{ visits: a, lt: 1 }",
            "{ and: [] }",
            "{ visits: a, equals: 1.5 }",
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
            eval("{ matches: '(' }"),
            Err(CondError::Regex { .. })
        ));
    }

    #[test]
    fn display_is_the_graph_note_form() {
        assert_eq!(
            parse("{ visits: implement, less_than: { data: max_rounds } }").to_string(),
            "visits implement less_than data.max_rounds"
        );
        assert_eq!(parse("{ matches: '^ok$' }").to_string(), "matches '^ok$'");
        assert_eq!(
            parse("{ data: strict, equals: true }").to_string(),
            "data strict equals true"
        );
    }

    #[test]
    fn serializes_as_written() {
        let cond = parse("{ visits: implement, less_than: { data: max_rounds } }");
        assert_eq!(
            serde_json::to_value(&cond).unwrap(),
            serde_json::json!({ "visits": "implement", "less_than": { "data": "max_rounds" } })
        );
    }
}
