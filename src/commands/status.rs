//! `decree status [<id>] [--cron]` (spec section 8). Reads runs and queues; never touches
//! a run.
//!
//! - No id: the runs by status, with the script each `active` run is running now (from
//!   `.running`, section 6) and the wait of each `waiting` run, then the queued messages.
//! - With id: the run's frontmatter, status, and its events as a table.
//! - `--cron`: the cron files and when each fires next.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use colored::Colorize;
use serde_json::{Map, Value};

use crate::commands::check::{md_files, Project};
use crate::commands::process::{context, run_ids, run_status};
use crate::config::{AppConfig, DECREE_DIR, INBOX_DIR, MIGRATIONS_DIR, RUNS_DIR};
use crate::cron;
use crate::error::DecreeError;
use crate::interpreter::{current_state, Context, RunStatus};
use crate::message::Message;
use crate::runtime::{Running, MESSAGE_FILE};

type Event = Map<String, Value>;

/// Run `decree status`.
pub fn run(project_root: &Path, id: Option<&str>, cron: bool) -> Result<(), DecreeError> {
    if cron {
        return show_cron(project_root);
    }
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    match id {
        Some(id) => show_run(&ctx, id),
        None => overview(&ctx, &project),
    }
}

/// One run, as the overview lists it.
struct Row {
    id: String,
    machine: String,
    state: String,
    detail: Option<String>,
}

/// Counts and lists of runs by status, then of queued messages.
fn overview(ctx: &Context, project: &Project) -> Result<(), DecreeError> {
    let mut groups: Vec<(RunStatus, Row)> = Vec::new();
    let mut finished: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    let ids = run_ids(&ctx.runs_dir())?;
    for id in &ids {
        let (status, events) = run_status(ctx, id)?;
        let state = current_state(&events).unwrap_or("-").to_string();
        let run_dir = ctx.runs_dir().join(id);
        let detail = match status {
            RunStatus::Active => Running::read(&run_dir)?.map(|r| running_line(id, &r)),
            RunStatus::Waiting => events.last().map(wait_line),
            RunStatus::Interrupted => Some(format!("continue with `decree retry {id}`")),
            RunStatus::Finished | RunStatus::Pending => None,
        };
        let row = Row {
            id: id.clone(),
            machine: field(events.first(), "machine").to_string(),
            state: state.clone(),
            detail,
        };
        match status {
            RunStatus::Finished => finished.entry(state).or_default().push(row),
            _ => groups.push((status, row)),
        }
    }

    println!("{} {}", "Runs:".bold(), ids.len());
    for status in [
        RunStatus::Active,
        RunStatus::Waiting,
        RunStatus::Pending,
        RunStatus::Interrupted,
    ] {
        let rows: Vec<&Row> = groups
            .iter()
            .filter(|(s, _)| *s == status)
            .map(|(_, row)| row)
            .collect();
        println!("  {}: {}", status.as_str(), rows.len());
        print_rows(&rows, "    ");
    }
    let total: usize = finished.values().map(Vec::len).sum();
    println!("  {}: {total}", RunStatus::Finished.as_str());
    for (state, rows) in &finished {
        println!("    {state}: {}", rows.len());
        print_rows(&rows.iter().collect::<Vec<_>>(), "      ");
    }

    println!("{}", "Queued:".bold());
    let inbox = md_files(&project.decree_dir.join(INBOX_DIR))?;
    println!("  {INBOX_DIR}/: {}", inbox.len());
    for file in &inbox {
        println!("    {file}");
    }
    let pending = project.pending_migrations()?;
    println!("  {MIGRATIONS_DIR}/: {} pending", pending.len());
    for file in &pending {
        println!("    {file}");
    }
    Ok(())
}

fn print_rows(rows: &[&Row], indent: &str) {
    for row in rows {
        println!(
            "{indent}{}  {}  `{}`",
            row.id.dimmed(),
            row.machine,
            row.state
        );
        if let Some(detail) = &row.detail {
            println!("{indent}  {detail}");
        }
    }
}

/// The script an `active` run is running now: name, pid, how long, and log path.
fn running_line(id: &str, r: &Running) -> String {
    let elapsed = DateTime::parse_from_rfc3339(&r.started_at)
        .map(|t| {
            (Utc::now() - t.with_timezone(&Utc))
                .num_milliseconds()
                .max(0) as u64
        })
        .map_or_else(|_| "?".to_string(), duration);
    format!(
        "running {} ({} of `{}`), pid {}, for {elapsed}, log {DECREE_DIR}/{RUNS_DIR}/{id}/{}",
        r.script, r.phase, r.state, r.pid, r.log
    )
}

/// The wait of a `waiting` run, from its last event: its wait id and options, or the
/// child run it waits for.
fn wait_line(last: &Event) -> String {
    let child = field(Some(last), "child");
    if !child.is_empty() {
        return format!("waiting for child run {child}");
    }
    format!(
        "wait id {}, options: {}",
        field(Some(last), "wait_id"),
        strings(last.get("options")).join(", ")
    )
}

/// One run: frontmatter, status, and the events as a table.
fn show_run(ctx: &Context, id: &str) -> Result<(), DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    if id.is_empty() || id.contains('/') || !run_dir.is_dir() {
        eprintln!("no run {id} in {DECREE_DIR}/{RUNS_DIR}/");
        return Ok(());
    }
    let (status, events) = run_status(ctx, id)?;
    println!("{} {id}", "Run".bold());
    println!("  machine: {}", field(events.first(), "machine"));
    let state = current_state(&events).unwrap_or("-");
    println!("  status: {} in `{state}`", status.as_str());
    if let Some(r) = Running::read(&run_dir)? {
        println!("  {}", running_line(id, &r));
    }
    if status == RunStatus::Waiting {
        if let Some(last) = events.last() {
            println!("  {}", wait_line(last));
        }
    }

    println!("{}", "Frontmatter:".bold());
    match Message::read(&run_dir.join(MESSAGE_FILE)) {
        Ok(m) => {
            let yaml = serde_norway::to_string(&m.frontmatter)?;
            for line in yaml.lines() {
                println!("  {line}");
            }
        }
        Err(e) => println!("  ({e})"),
    }

    println!("{}", "Events:".bold());
    println!("  {:<4} {:<12} {:<12} DETAIL", "SEQ", "TIME", "TYPE");
    for e in &events {
        let ts = field(Some(e), "ts");
        let time = ts.get(11..23).unwrap_or(ts);
        let kind = field(Some(e), "type");
        println!(
            "  {:<4} {:<12} {:<12} {}",
            e.get("seq").map_or_else(String::new, Value::to_string),
            time,
            kind,
            event_detail(kind, e)
        );
    }
    Ok(())
}

/// The DETAIL column for one event (section 7, events.jsonl).
fn event_detail(kind: &str, e: &Event) -> String {
    let f = |key: &str| field(Some(e), key);
    let ms = |key: &str| e.get(key).and_then(Value::as_u64).map(duration);
    let mut out = match kind {
        "transition" => {
            let from = e.get("from").and_then(Value::as_str).unwrap_or("·");
            let mut s = format!("{from} --{}--> {} ({})", f("event"), f("to"), f("source"));
            if let Some(code) = e.get("exit_code").and_then(Value::as_i64) {
                s.push_str(&format!(", exit {code}"));
            }
            s
        }
        "script" => {
            let exit = e
                .get("exit_code")
                .and_then(Value::as_i64)
                .map_or_else(|| "killed".to_string(), |c| format!("exit {c}"));
            let mut s = format!(
                "{}/{} ({}, attempt {}) {exit} in {}",
                f("state"),
                f("script"),
                f("phase"),
                e.get("attempt").map_or_else(String::new, Value::to_string),
                ms("duration_ms").unwrap_or_default()
            );
            if e.get("timed_out") == Some(&Value::Bool(true)) {
                s.push_str(", timed out");
            }
            s.push_str(&format!(", log {}", f("log")));
            s
        }
        "decision" => {
            let mut s = format!("{}: {} → {}", f("state"), f("kind"), f("event"));
            match f("kind") {
                "model" => {
                    s.push_str(&format!(
                        " (router {}, run {}, pick {}",
                        f("router"),
                        f("child_run"),
                        f("pick")
                    ));
                    if let Some(c) = e.get("confidence").and_then(Value::as_f64) {
                        s.push_str(&format!(", confidence {c}"));
                    }
                    if let Some(d) = ms("duration_ms") {
                        s.push_str(&format!(", {d}"));
                    }
                    s.push(')');
                }
                "person" => s.push_str(&format!(" (reply {})", f("reply"))),
                _ => {}
            }
            s
        }
        "waiting" => format!("{}: {}", f("state"), wait_line(e)),
        "received" => {
            let child = f("child");
            if child.is_empty() {
                let mut s = format!("{} for {}", f("event"), f("wait_id"));
                if !f("file").is_empty() {
                    s.push_str(&format!(" (reply {})", f("file")));
                }
                if e.get("timed_out") == Some(&Value::Bool(true)) {
                    s.push_str(", timed out");
                }
                s
            } else {
                format!("child run {child} ended: {}", f("event"))
            }
        }
        "run_finished" => format!(
            "{} after {}",
            f("state"),
            ms("duration_ms").unwrap_or_default()
        ),
        "interrupted" => {
            let mut s = format!("{}: {}", f("state"), f("cause"));
            if !f("script").is_empty() {
                s.push_str(&format!(", script {}", f("script")));
            }
            s
        }
        _ => String::new(),
    };
    for key in ["invalid_event", "error", "reason", "router_error"] {
        if !f(key).is_empty() {
            out.push_str(&format!(", {key}: {}", f(key)));
        }
    }
    if let Some(failures) = e.get("exit_failures") {
        out.push_str(&format!(
            ", exit_failures: {}",
            strings(Some(failures)).join(", ")
        ));
    }
    out
}

/// `--cron`: each cron file, its schedule and machine, and when it fires next.
fn show_cron(project_root: &Path) -> Result<(), DecreeError> {
    let config = AppConfig::load_from_project(project_root)?;
    let mut files = cron::scan_cron_files(project_root)?;
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    if files.is_empty() {
        println!("No cron files.");
        return Ok(());
    }
    println!(
        "{:<32}{:<19}{:<16}NEXT RUN",
        "CRON FILE", "SCHEDULE", "MACHINE"
    );
    for cf in &files {
        let machine = cf
            .machine
            .as_deref()
            .or(config.default_machine.as_deref())
            .unwrap_or("\u{2014}");
        let next = cf.schedule.upcoming(Local).next().map_or_else(
            || "\u{2014}".to_string(),
            |t| format!("{} (in {})", t.format("%Y-%m-%d %H:%M"), countdown(t)),
        );
        println!(
            "{:<32}{:<19}{:<16}{next}",
            cf.filename, cf.cron_expr, machine
        );
    }
    Ok(())
}

/// Time left until `t`: "30s", "5m", "2h", "3d".
fn countdown(t: DateTime<Local>) -> String {
    let secs = (t - Local::now()).num_seconds().max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// A duration in milliseconds: "850ms", "4.2s", "3m 04s", "1h 02m".
fn duration(ms: u64) -> String {
    let secs = ms / 1000;
    match ms {
        0..1000 => format!("{ms}ms"),
        1000..60_000 => format!("{}.{}s", secs, (ms % 1000) / 100),
        60_000..3_600_000 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

fn field<'e>(e: Option<&'e Event>, key: &str) -> &'e str {
    e.and_then(|e| e.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn strings(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(v: Value) -> Event {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn test_duration() {
        assert_eq!(duration(850), "850ms");
        assert_eq!(duration(4_230), "4.2s");
        assert_eq!(duration(184_000), "3m 04s");
        assert_eq!(duration(3_720_000), "1h 02m");
    }

    #[test]
    fn test_countdown() {
        let now = Local::now();
        assert_eq!(countdown(now + chrono::Duration::seconds(90)), "1m");
        assert_eq!(countdown(now + chrono::Duration::seconds(3700)), "1h");
        assert_eq!(countdown(now + chrono::Duration::seconds(86500)), "1d");
        assert!(countdown(now + chrono::Duration::seconds(30)).ends_with('s'));
    }

    #[test]
    fn test_event_detail_per_type() {
        let cases = [
            (
                json!({"type": "transition", "from": null, "event": "claimed", "to": "work",
                       "source": "claim", "exit_code": null}),
                "· --claimed--> work (claim)",
            ),
            (
                json!({"type": "script", "state": "work", "phase": "invoke", "script": "build",
                       "attempt": 2, "duration_ms": 4230, "exit_code": 0, "log": "0002-work-build.log"}),
                "work/build (invoke, attempt 2) exit 0 in 4.2s, log 0002-work-build.log",
            ),
            (
                json!({"type": "decision", "state": "route", "kind": "model", "event": "fix",
                       "router": "claude_router", "child_run": "c1", "pick": "fix",
                       "confidence": 0.9, "duration_ms": 1500, "reason": "tests fail"}),
                "route: model → fix (router claude_router, run c1, pick fix, confidence 0.9, 1.5s), reason: tests fail",
            ),
            (
                json!({"type": "waiting", "state": "approval", "wait_id": "r.w3",
                       "options": ["approve", "reject"]}),
                "approval: wait id r.w3, options: approve, reject",
            ),
            (
                json!({"type": "received", "wait_id": "r.w3", "event": "approve", "file": "x.md"}),
                "approve for r.w3 (reply x.md)",
            ),
            (
                json!({"type": "interrupted", "state": "work", "cause": "signal", "script": "build"}),
                "work: signal, script build",
            ),
        ];
        for (e, want) in cases {
            let e = event(e);
            assert_eq!(event_detail(field(Some(&e), "type"), &e), want);
        }
    }

    #[test]
    fn test_running_line_names_script_pid_elapsed_and_log() {
        let r = Running {
            pid: 4242,
            state: "work".into(),
            phase: "invoke".into(),
            script: "build".into(),
            started_at: crate::runtime::timestamp(Utc::now() - chrono::Duration::seconds(3)),
            log: "0001-work-build.log".into(),
        };
        let line = running_line("r1", &r);
        assert!(
            line.starts_with("running build (invoke of `work`), pid 4242, for 3."),
            "{line}"
        );
        assert!(line.ends_with(", log .decree/runs/r1/0001-work-build.log"));
    }
}
