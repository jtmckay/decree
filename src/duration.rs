//! The one duration format (docs/reference/machines.md, Durations): a whole number followed
//! by one unit, `s`, `m`, `h` or `d` (`90s`, `10m`, `12h`, `7d`), as Kubernetes and Go
//! durations write them, without fractions or combinations. A machine's `timeout`,
//! `decree prune --older-than` and `decree daemon --interval` all parse it here.

use std::time::Duration;

use serde::{Deserialize, Deserializer};

/// At most this many digits, as the schema's pattern says, so the schema and the parser
/// accept the same strings. 999999999d is far inside what `chrono::TimeDelta` holds, so a
/// parsed duration can always be added to a timestamp without a second check.
const MAX_DIGITS: usize = 9;

/// Parse a duration: a whole number followed by `s`, `m`, `h` or `d`.
pub fn parse(text: &str) -> Result<Duration, String> {
    let bad = || {
        format!(
            "`{text}` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d"
        )
    };
    let (digits, unit) = match text.char_indices().last() {
        Some((i, unit)) => (&text[..i], unit),
        None => return Err(bad()),
    };
    let secs_per_unit = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => return Err(bad()),
    };
    if digits.is_empty() || digits.len() > MAX_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad());
    }
    let n: u64 = digits.parse().map_err(|_| bad())?;
    Ok(Duration::from_secs(n * secs_per_unit))
}

/// Deserialize an optional `timeout`: any scalar, so that a bare number fails with the
/// format rather than a type error (V16).
pub fn deserialize_timeout<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error> {
    let value = serde_norway::Value::deserialize(deserializer)?;
    let text = match &value {
        serde_norway::Value::String(s) => s.clone(),
        other => serde_norway::to_string(other)
            .unwrap_or_default()
            .trim_end()
            .to_string(),
    };
    parse(&text)
        .map(Some)
        .map_err(|e| serde::de::Error::custom(format!("timeout: {e} (V16)")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_takes_a_whole_number_and_one_unit() {
        assert_eq!(parse("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(parse("10m"), Ok(Duration::from_secs(600)));
        assert_eq!(parse("12h"), Ok(Duration::from_secs(12 * 3600)));
        assert_eq!(parse("7d"), Ok(Duration::from_secs(7 * 86400)));
        assert_eq!(parse("0s"), Ok(Duration::ZERO));
    }

    #[test]
    fn test_parse_rejects_everything_else() {
        for bad in [
            "",
            "1.5h",
            "1h30m",
            "10",
            "-1m",
            "+1m",
            "1w",
            "h",
            "1 h",
            "1H",
            "1é",
            "1000000000d",
            "é",
            "99999999999999999999s",
            "9999999999999999d",
        ] {
            let err = parse(bad).unwrap_err();
            assert!(err.contains("is not a duration"), "{bad}: {err}");
        }
    }
}
