//! `.decree/env` (docs/reference/scripts.md, Environment): project variables every script
//! gets, in the dotenv format Docker Compose's `env_file` and systemd's `EnvironmentFile`
//! read. One `KEY=value` per line; blank lines and `#` lines are ignored; `export ` before a
//! key is allowed; a value wrapped in single or double quotes loses them. No variable
//! expansion, no multi-line values. Also the key rules of an invoke's `env` map.

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

/// The variables of a dotenv file's `text`, in file order (a later line wins over an
/// earlier one with the same key), or every malformed line as `(line number, message)`.
pub fn parse(text: &str) -> Result<Vars, LineErrors> {
    let mut vars = Vec::new();
    let mut errors = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        match parse_line(raw) {
            Ok(Some(var)) => vars.push(var),
            Ok(None) => {}
            Err(message) => errors.push((i + 1, message)),
        }
    }
    if errors.is_empty() {
        Ok(vars)
    } else {
        Err(errors)
    }
}

/// One line: `None` for a blank or comment line.
fn parse_line(raw: &str) -> Result<Option<(String, String)>, String> {
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
    let value = match value.chars().next() {
        Some(quote @ ('"' | '\'')) => value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
            .ok_or_else(|| {
                format!("the value of `{key}` opens a {quote} quote it does not close")
            })?,
        _ => value,
    };
    Ok(Some((key.to_string(), value.to_string())))
}

/// `.decree/env` under `decree_dir`, parsed: `Ok(Ok(vars))`, empty when there is no file,
/// or `Ok(Err(errors))` for malformed lines.
pub fn read(decree_dir: &Path) -> io::Result<Result<Vars, LineErrors>> {
    match std::fs::read(decree_dir.join(crate::layout::ENV_FILE)) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Ok(parse(&text)),
            Err(_) => Ok(Err(vec![(1, "file is not valid UTF-8".to_string())])),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Ok(Vec::new())),
        Err(e) => Err(e),
    }
}

/// The variables scripts get from the file: those not set in decree's own environment,
/// which wins, as with Docker Compose, so a deployment can override the file.
pub fn unset_in_process(vars: Vars) -> Vars {
    vars.into_iter()
        .filter(|(key, _)| std::env::var_os(key).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(text: &str) -> Vec<(String, String)> {
        parse(text).unwrap()
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
    fn values_are_literal_no_expansion_no_inline_comment() {
        assert_eq!(
            vars("A=$HOME/x\nB=a=b # not a comment\nC=\"it's\"\nD='say \"hi\"'\n"),
            vec![
                pair("A", "$HOME/x"),
                pair("B", "a=b # not a comment"),
                pair("C", "it's"),
                pair("D", "say \"hi\""),
            ]
        );
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
        assert_eq!(read(dir.path()).unwrap(), Ok(vec![pair("A", "1")]));
    }

    #[test]
    fn the_process_environment_wins() {
        // `PATH` is set in every test process; the other key is not.
        let kept = unset_in_process(vec![
            pair("PATH", "/nowhere"),
            pair("DECREE_TEST_DOTENV_UNSET_KEY", "x"),
        ]);
        assert_eq!(kept, vec![pair("DECREE_TEST_DOTENV_UNSET_KEY", "x")]);
    }
}
