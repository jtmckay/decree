//! `.decree/env` (docs/reference/scripts.md, Environment): project variables every script
//! gets, in the dotenv format Docker Compose's `env_file` and systemd's `EnvironmentFile`
//! read. One `KEY=value` per line; blank lines and `#` lines are ignored; `export ` before a
//! key is allowed; a value wrapped in single or double quotes loses them. Values are
//! interpolated as Compose does, the subset `${VAR}`, `$VAR`, `${VAR:-default}`,
//! `${VAR-default}` and `$$`; single-quoted values stay literal. No multi-line values.
//! Also the key rules of an invoke's `env` map.

use std::io;
use std::path::Path;

/// Variables in file order, `(key, value)`.
pub type Vars = Vec<(String, String)>;

/// Malformed lines, `(line number from 1, message)`.
pub type LineErrors = Vec<(usize, String)>;

/// The pattern every key matches, as messages print it.
pub const KEY_PATTERN: &str = "^[A-Za-z_][A-Za-z0-9_]*$";

/// Whether `key` matches `KEY_PATTERN`.
pub fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `key` belongs to decree: `DECREE_*`, `TRACEPARENT` or `TRACESTATE`.
pub fn is_reserved(key: &str) -> bool {
    key.starts_with("DECREE_")
        || key == crate::trace::TRACEPARENT_ENV
        || key == crate::trace::TRACESTATE_ENV
}

/// What is wrong with `key`, if anything: it must match `KEY_PATTERN` and not be reserved.
pub fn key_error(key: &str) -> Option<String> {
    if !is_key(key) {
        Some(format!("key `{key}` does not match `{KEY_PATTERN}`"))
    } else if is_reserved(key) {
        Some(format!(
            "key `{key}` is reserved: `DECREE_*`, `TRACEPARENT` and `TRACESTATE` belong to decree"
        ))
    } else {
        None
    }
}

/// One `KEY=value` line of a dotenv file, its value not yet interpolated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Line number, from 1.
    pub line: usize,
    pub key: String,
    value: Vec<Part>,
}

/// A piece of a value: literal text, or a variable to look up.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    /// `$VAR` or `${VAR}`, with the default of `${VAR:-default}` or `${VAR-default}`.
    Var {
        name: String,
        default: Option<Fallback>,
    },
}

/// The default of `${VAR:-default}` (`if_empty`: also when `VAR` is empty) or
/// `${VAR-default}` (only when it is unset). It is interpolated in turn, as in Compose.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fallback {
    if_empty: bool,
    value: Vec<Part>,
}

/// The lines of a dotenv file's `text`, in file order, or every malformed line as
/// `(line number, message)`.
pub fn parse(text: &str) -> Result<Vec<Entry>, LineErrors> {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        match parse_line(raw) {
            Ok(Some((key, value))) => entries.push(Entry {
                line: i + 1,
                key,
                value,
            }),
            Ok(None) => {}
            Err(message) => errors.push((i + 1, message)),
        }
    }
    if errors.is_empty() {
        Ok(entries)
    } else {
        Err(errors)
    }
}

/// One line: `None` for a blank or comment line.
fn parse_line(raw: &str) -> Result<Option<(String, Vec<Part>)>, String> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let pair = line.strip_prefix("export ").map_or(line, str::trim_start);
    let Some((key, value)) = pair.split_once('=') else {
        return Err(format!("`{line}` is not `KEY=value`"));
    };
    let key = key.trim_end();
    if let Some(message) = key_error(key) {
        return Err(message);
    }
    let value = value.trim_start();
    let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''));
    let unquoted = match quote {
        Some(quote) => value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
            .ok_or_else(|| {
                format!("the value of `{key}` opens a {quote} quote it does not close")
            })?,
        None => value,
    };
    // Single-quoted values are literal, as in Compose.
    let parts = if quote == Some('\'') {
        vec![Part::Text(unquoted.to_string())]
    } else {
        let mut chars = unquoted.char_indices().peekable();
        let parts = parse_parts(unquoted, &mut chars, None)
            .map_err(|e| format!("the value of `{key}`: {e}"))?;
        parts
    };
    Ok(Some((key.to_string(), parts)))
}

type Chars<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// The parts of `text` from `chars` on: to the end, or, in the default of a `${VAR:-…}`
/// opened at byte `opened`, to the `}` that closes it (consumed).
fn parse_parts(text: &str, chars: &mut Chars, opened: Option<usize>) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    loop {
        let Some((i, c)) = chars.next() else {
            if let Some(start) = opened {
                return Err(unclosed(text, start));
            }
            break;
        };
        match c {
            '}' if opened.is_some() => break,
            '$' => match chars.peek().map(|&(_, c)| c) {
                Some('$') => {
                    chars.next();
                    literal.push('$');
                }
                Some('{') => {
                    chars.next();
                    flush(&mut literal, &mut parts);
                    parts.push(parse_braced(text, i, chars)?);
                }
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                    flush(&mut literal, &mut parts);
                    let name = take_name(chars);
                    parts.push(Part::Var {
                        name,
                        default: None,
                    });
                }
                // A `$` that starts nothing stays a `$`.
                _ => literal.push('$'),
            },
            c => literal.push(c),
        }
    }
    flush(&mut literal, &mut parts);
    Ok(parts)
}

fn flush(literal: &mut String, parts: &mut Vec<Part>) {
    if !literal.is_empty() {
        parts.push(Part::Text(std::mem::take(literal)));
    }
}

/// The longest name at `chars`: `[A-Za-z0-9_]*`.
fn take_name(chars: &mut Chars) -> String {
    let mut name = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            break;
        }
        name.push(c);
        chars.next();
    }
    name
}

fn unclosed(text: &str, start: usize) -> String {
    format!("`{}` is not closed by `}}`", &text[start..])
}

/// What follows `${` (at byte `start` of `text`): `VAR}`, `VAR:-default}` or `VAR-default}`.
fn parse_braced(text: &str, start: usize, chars: &mut Chars) -> Result<Part, String> {
    let name = take_name(chars);
    if !is_key(&name) {
        return Err(match chars.peek() {
            None => unclosed(text, start),
            Some(_) => format!(
                "`{}` does not start with a variable name matching `{KEY_PATTERN}`",
                &text[start..]
            ),
        });
    }
    let malformed = |rest: &str| {
        format!("`${{{name}{rest}` is not `${{{name}}}`, `${{{name}:-default}}` or `${{{name}-default}}`")
    };
    match chars.next().map(|(_, c)| c) {
        None => Err(unclosed(text, start)),
        Some('}') => Ok(Part::Var {
            name,
            default: None,
        }),
        Some(':') => match chars.next().map(|(_, c)| c) {
            Some('-') => Ok(Part::Var {
                default: Some(Fallback {
                    if_empty: true,
                    value: parse_parts(text, chars, Some(start))?,
                }),
                name,
            }),
            None => Err(unclosed(text, start)),
            Some(c) => Err(malformed(&format!(":{c}…"))),
        },
        Some('-') => Ok(Part::Var {
            default: Some(Fallback {
                if_empty: false,
                value: parse_parts(text, chars, Some(start))?,
            }),
            name,
        }),
        Some(c) => Err(malformed(&format!("{c}…"))),
    }
}

/// What scripts get from a parsed file, and what `decree check` warns about.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The interpolated variables, in file order (a later line wins over an earlier one
    /// with the same key), without those set in decree's own environment, which wins, as
    /// with Docker Compose, so a deployment can override the file.
    pub vars: Vars,
    /// Each variable referenced without a default and set nowhere, `(line, name)`; it
    /// reads as empty, as in Compose.
    pub unset: Vec<(usize, String)>,
}

/// Interpolate `entries`: `VAR` is looked up with `process` (decree's own environment)
/// first, then in the lines above, as their effective values.
pub fn resolve(entries: &[Entry], process: impl Fn(&str) -> Option<String>) -> Resolved {
    let mut effective: std::collections::HashMap<&str, String> = Default::default();
    let mut out = Resolved::default();
    for entry in entries {
        let lookup = |name: &str| process(name).or_else(|| effective.get(name).cloned());
        let value = interpolate(&entry.value, &lookup, &mut |name| {
            out.unset.push((entry.line, name.to_string()));
        });
        match process(&entry.key) {
            Some(set) => {
                effective.insert(&entry.key, set);
            }
            None => {
                effective.insert(&entry.key, value.clone());
                out.vars.push((entry.key.clone(), value));
            }
        }
    }
    out
}

fn interpolate(
    parts: &[Part],
    lookup: &dyn Fn(&str) -> Option<String>,
    unset: &mut dyn FnMut(&str),
) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            Part::Text(text) => out.push_str(text),
            Part::Var { name, default } => match (lookup(name), default) {
                (Some(value), Some(d)) if d.if_empty && value.is_empty() => {
                    out.push_str(&interpolate(&d.value, lookup, unset));
                }
                (Some(value), _) => out.push_str(&value),
                (None, Some(d)) => out.push_str(&interpolate(&d.value, lookup, unset)),
                (None, None) => unset(name),
            },
        }
    }
    out
}

/// Decree's own environment, as `resolve` looks it up.
pub fn process_var(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

/// `.decree/env` under `decree_dir`, parsed: `Ok(Ok(entries))`, empty when there is no
/// file, or `Ok(Err(errors))` for malformed lines.
pub fn read(decree_dir: &Path) -> io::Result<Result<Vec<Entry>, LineErrors>> {
    match std::fs::read(decree_dir.join(crate::layout::ENV_FILE)) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Ok(parse(&text)),
            Err(_) => Ok(Err(vec![(1, "file is not valid UTF-8".to_string())])),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Ok(Vec::new())),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The variables of `text` with no process environment.
    fn vars(text: &str) -> Vec<(String, String)> {
        with_process(text, &[]).vars
    }

    /// `text` resolved with `process` as decree's environment.
    fn with_process(text: &str, process: &[(&str, &str)]) -> Resolved {
        let entries = parse(text).unwrap();
        resolve(&entries, |name| {
            process
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        })
    }

    fn pair(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn plain_quoted_and_exported_values() {
        let text = "A=1\nB=\"http://box:8188\"\nC='two words'\nexport D=x\n  E = spaced  \nF=\n";
        assert_eq!(
            vars(text),
            vec![
                pair("A", "1"),
                pair("B", "http://box:8188"),
                pair("C", "two words"),
                pair("D", "x"),
                pair("E", "spaced"),
                pair("F", ""),
            ]
        );
    }

    #[test]
    fn blank_and_comment_lines_are_ignored() {
        assert_eq!(
            vars("\n# a comment\n   \n  # indented\nA=1\r\n"),
            vec![pair("A", "1")]
        );
    }

    #[test]
    fn no_inline_comment_and_quotes_inside_stay() {
        assert_eq!(
            vars("B=a=b # not a comment\nC=\"it's\"\nD='say \"hi\"'\n"),
            vec![
                pair("B", "a=b # not a comment"),
                pair("C", "it's"),
                pair("D", "say \"hi\""),
            ]
        );
    }

    #[test]
    fn braced_and_bare_references_take_a_line_above() {
        let text = "HOST=box\nA=http://${HOST}:8188\nB=\"$HOST/x\"\nC=$HOST.lan\n";
        assert_eq!(
            vars(text),
            vec![
                pair("HOST", "box"),
                pair("A", "http://box:8188"),
                pair("B", "box/x"),
                pair("C", "box.lan"),
            ]
        );
    }

    #[test]
    fn a_bare_name_is_the_longest_run_of_name_characters() {
        let text = "H=box\nHOST=other\nA=$HOST_x\nB=$HOST-x\n";
        let resolved = with_process(text, &[]);
        assert_eq!(resolved.vars[2], pair("A", ""));
        assert_eq!(resolved.vars[3], pair("B", "other-x"));
        assert_eq!(resolved.unset, vec![(3, "HOST_x".to_string())]);
    }

    #[test]
    fn colon_dash_default_when_unset_or_empty() {
        let text = "E=\nS=set\nA=${U:-d}\nB=${E:-d}\nC=${S:-d}\n";
        assert_eq!(
            &vars(text)[2..],
            &[pair("A", "d"), pair("B", "d"), pair("C", "set")]
        );
    }

    #[test]
    fn dash_default_only_when_unset() {
        let text = "E=\nS=set\nA=${U-d}\nB=${E-d}\nC=${S-d}\nD=${U-}\n";
        assert_eq!(
            &vars(text)[2..],
            &[
                pair("A", "d"),
                pair("B", ""),
                pair("C", "set"),
                pair("D", "")
            ]
        );
    }

    #[test]
    fn a_default_is_interpolated_and_has_no_warning() {
        let text = "HOST=box\nA=${URL:-http://${HOST}:1}\nB=${U:-$$}\nC=${U-a:b-c}\n";
        let resolved = with_process(text, &[]);
        assert_eq!(
            &resolved.vars[1..],
            &[
                pair("A", "http://box:1"),
                pair("B", "$"),
                pair("C", "a:b-c")
            ]
        );
        assert!(resolved.unset.is_empty(), "{:?}", resolved.unset);
    }

    #[test]
    fn double_dollar_is_a_dollar_and_a_lone_dollar_stays() {
        assert_eq!(
            vars("C=$$5\nD=\"$${B}\"\nE=cost $ 5$\nF=$1\n"),
            vec![
                pair("C", "$5"),
                pair("D", "${B}"),
                pair("E", "cost $ 5$"),
                pair("F", "$1"),
            ]
        );
    }

    #[test]
    fn single_quoted_values_are_literal() {
        let resolved = with_process("B=x\nA='${B}'\nC='$B $$ ${B:-d} ${'\n", &[("B", "p")]);
        assert_eq!(
            resolved.vars,
            vec![pair("A", "${B}"), pair("C", "$B $$ ${B:-d} ${")]
        );
        assert!(resolved.unset.is_empty());
    }

    #[test]
    fn the_process_environment_wins_for_a_reference() {
        let text = "HOST=box\nURL=http://${HOST}:8188\n";
        assert_eq!(
            with_process(text, &[]).vars,
            vec![pair("HOST", "box"), pair("URL", "http://box:8188")]
        );
        assert_eq!(
            with_process(text, &[("HOST", "other")]).vars,
            vec![pair("URL", "http://other:8188")]
        );
        // Not only for keys of the file: any variable of decree's environment.
        assert_eq!(
            with_process("A=${HOME}/x\n", &[("HOME", "/h")]).vars,
            vec![pair("A", "/h/x")]
        );
    }

    #[test]
    fn the_process_environment_wins_for_a_key() {
        let resolved = with_process("URL=http://file\nA=${URL}/x\n", &[("URL", "http://env")]);
        assert_eq!(resolved.vars, vec![pair("A", "http://env/x")]);
    }

    #[test]
    fn only_lines_above_count_and_a_later_line_wins() {
        let resolved = with_process("A=$B\nB=1\nC=$B\nB=2\nD=$B\nB=${B}3\n", &[]);
        assert_eq!(
            resolved.vars,
            vec![
                pair("A", ""),
                pair("B", "1"),
                pair("C", "1"),
                pair("B", "2"),
                pair("D", "2"),
                pair("B", "23"),
            ]
        );
        assert_eq!(resolved.unset, vec![(1, "B".to_string())]);
    }

    #[test]
    fn an_unknown_variable_is_empty_with_a_warning() {
        let resolved = with_process("# hosts\n\nA=1\nURL=http://${HOST}:$PORT/\n", &[]);
        assert_eq!(resolved.vars[1], pair("URL", "http://:/"));
        assert_eq!(
            resolved.unset,
            vec![(4, "HOST".to_string()), (4, "PORT".to_string())]
        );
    }

    #[test]
    fn an_unterminated_or_malformed_reference_is_malformed() {
        let cases = [
            ("A=${HOST", "`${HOST` is not closed by `}`"),
            ("A=\"x${\"", "`${` is not closed by `}`"),
            ("A=${HOST:-x", "`${HOST:-x` is not closed by `}`"),
            ("A=${HOST-${B}", "`${HOST-${B}` is not closed by `}`"),
            ("A=${HOST:", "`${HOST:` is not closed by `}`"),
            ("A=${HOST:x}", "`${HOST:x…` is not `${HOST}`"),
            ("A=${HOST:?err}", "`${HOST:?…` is not `${HOST}`"),
            ("A=${HOST:+x}", "`${HOST:+…` is not `${HOST}`"),
            ("A=${HOST?err}", "`${HOST?…` is not `${HOST}`"),
            ("A=${}", "`${}` does not start with a variable name"),
            ("A=${1A}", "`${1A}` does not start with a variable name"),
            ("A=${:-x}", "`${:-x}` does not start with a variable name"),
            ("A=${B:-${C:x}}", "`${C:x…` is not `${C}`"),
        ];
        for (text, expected) in cases {
            let errors = parse(text).unwrap_err();
            assert_eq!(errors.len(), 1, "{text}: {errors:?}");
            assert_eq!(errors[0].0, 1, "{text}");
            assert!(
                errors[0].1.starts_with("the value of `A`: "),
                "{text}: {errors:?}"
            );
            assert!(errors[0].1.contains(expected), "{text}: {errors:?}");
        }
    }

    #[test]
    fn a_line_without_equals_is_malformed() {
        assert_eq!(
            parse("A=1\nnot a pair\n").unwrap_err(),
            vec![(2, "`not a pair` is not `KEY=value`".to_string())]
        );
    }

    #[test]
    fn a_bad_key_or_unclosed_quote_is_malformed() {
        let errors = parse("1A=x\nA-B=x\n=x\nC=\"open\nD='open\"\n").unwrap_err();
        let lines: Vec<usize> = errors.iter().map(|(n, _)| *n).collect();
        assert_eq!(lines, vec![1, 2, 3, 4, 5], "{errors:?}");
        assert!(errors[0].1.contains(KEY_PATTERN), "{errors:?}");
        assert!(errors[3].1.contains("does not close"), "{errors:?}");
    }

    #[test]
    fn reserved_keys_are_errors() {
        let errors = parse("DECREE_X=1\nTRACEPARENT=x\nTRACESTATE=y\nDECREEX=ok\n").unwrap_err();
        let lines: Vec<usize> = errors.iter().map(|(n, _)| *n).collect();
        assert_eq!(lines, vec![1, 2, 3], "{errors:?}");
        assert!(errors[0].1.contains("reserved"), "{errors:?}");
    }

    #[test]
    fn a_missing_file_has_no_variables() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(read(dir.path()).unwrap(), Ok(Vec::new()));
        std::fs::write(dir.path().join("env"), "A='1'\n").unwrap();
        let entries = read(dir.path()).unwrap().unwrap();
        assert_eq!(resolve(&entries, |_| None).vars, vec![pair("A", "1")]);
    }

    #[test]
    fn process_var_reads_decree_s_environment() {
        // `PATH` is set in every test process; the other key is not.
        assert!(process_var("PATH").is_some());
        assert_eq!(process_var("DECREE_TEST_DOTENV_UNSET_KEY"), None);
    }
}
