//! `decree graph`: Mermaid diagrams drawn from the machine arena (spec section 9), so the
//! picture cannot drift from what the interpreter runs. Output is byte-for-byte
//! deterministic; decree renders no images.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::machine::{LoadedMachine, MACHINES_DIR};

/// Root-level final state an unhandled `error` goes to (section 5, Rules).
const FAILED: &str = "failed";

/// The Markdown document for one machine: heading, description and a `stateDiagram-v2`.
pub fn machine_document(m: &LoadedMachine) -> Result<String, String> {
    Ok(document(&m.id, m.description(), &state_diagram(m)?))
}

/// The Markdown document for the whole system: a `flowchart LR` of every machine, the
/// `emits` edges between them, and one node per cron file. `crons` is `(file stem, machine)`
/// in filename order.
pub fn system_document(
    machines: &BTreeMap<String, LoadedMachine>,
    crons: &[(String, String)],
) -> String {
    document(
        "All machines",
        "Every machine, the `emits` edges between them, and cron entry points.",
        &flowchart(machines, crons),
    )
}

fn document(heading: &str, description: &str, diagram: &str) -> String {
    format!("# {heading}\n\n{description}\n\n```mermaid\n{diagram}```\n")
}

/// One edge to draw, already placed in its container.
struct Line {
    source: String,
    event: String,
    target: String,
    label: String,
}

/// The `stateDiagram-v2` for one machine, every line ending in `\n`.
pub fn state_diagram(m: &LoadedMachine) -> Result<String, String> {
    let mut edges: BTreeMap<usize, Vec<Line>> = BTreeMap::new();
    for (i, node) in m.nodes.iter().enumerate().skip(1) {
        for edge in &node.transitions {
            let target = m.find(&edge.target).ok_or_else(|| {
                format!(
                    "{MACHINES_DIR}/{}.yml: {}: transition `{}` targets unknown state `{}` (run `decree check`)",
                    m.id,
                    m.state_path(i),
                    edge.event,
                    edge.target
                )
            })?;
            let mut label = edge.event.clone();
            if let Some(cond) = &edge.cond {
                let _ = write!(label, " [{cond}]");
            }
            if edge.event != "error" {
                if node.router.is_some() {
                    if node.default.as_deref() == Some(edge.event.as_str()) {
                        label.push_str(" (llm, default)");
                    } else {
                        label.push_str(" (llm)");
                    }
                }
                if is_waiting(m, i) {
                    label.push_str(" (external)");
                }
            }
            if edge.internal {
                label.push_str(" (internal)");
            }
            edges
                .entry(domain(m, i, target, edge.internal))
                .or_default()
                .push(Line {
                    source: node.id.clone(),
                    event: edge.event.clone(),
                    target: edge.target.clone(),
                    label,
                });
        }
        if implicit_error(m, i) {
            // `failed` is root-level, so the transition domain is the root.
            edges.entry(0).or_default().push(Line {
                source: node.id.clone(),
                event: "error".into(),
                target: FAILED.into(),
                label: "error (implicit)".into(),
            });
        }
    }
    for lines in edges.values_mut() {
        lines.sort_by(|a, b| (&a.source, &a.event).cmp(&(&b.source, &b.event)));
    }

    let mut out = String::from("stateDiagram-v2\n");
    container(m, 0, 1, &edges, &mut out);

    let root = m.root();
    if !root.onentry.is_empty() || !root.onexit.is_empty() {
        let initial = root.initial.as_deref().unwrap_or_default();
        push(&mut out, 1, &format!("note left of {initial}"));
        script_lines(&mut out, "machine onentry", &root.onentry);
        script_lines(&mut out, "machine onexit", &root.onexit);
        push(&mut out, 1, "end note");
    }
    // Arena order: depth-first, children in `BTreeMap` order.
    for node in m.nodes.iter().skip(1) {
        if !node.onentry.is_empty() || !node.onexit.is_empty() {
            push(&mut out, 1, &format!("note right of {}", node.id));
            script_lines(&mut out, "onentry", &node.onentry);
            script_lines(&mut out, "onexit", &node.onexit);
            push(&mut out, 1, "end note");
        }
    }
    Ok(out)
}

/// Section 9 steps 1–4 for container `c` (the root or a compound state) at `level`.
fn container(
    m: &LoadedMachine,
    c: usize,
    level: usize,
    edges: &BTreeMap<usize, Vec<Line>>,
    out: &mut String,
) {
    let node = &m.nodes[c];
    push(
        out,
        level,
        &format!("[*] --> {}", node.initial.as_deref().unwrap_or_default()),
    );
    for &child in &node.children {
        if m.is_compound(child) {
            push(out, level, &format!("state {} {{", m.nodes[child].id));
            container(m, child, level + 1, edges, out);
            push(out, level, "}");
        }
    }
    for line in edges.get(&c).into_iter().flatten() {
        let text = format!(
            "{} --> {}: {}",
            line.source,
            line.target,
            escape(&line.label)
        );
        push(out, level, &text);
    }
    for &child in &node.children {
        if m.nodes[child].is_final {
            push(out, level, &format!("{} --> [*]", m.nodes[child].id));
        }
    }
}

/// SCXML transition domain (section 5, Rules): the source itself for a `type: internal`
/// transition to one of its descendants, else the deepest proper ancestor of both ends.
fn domain(m: &LoadedMachine, source: usize, target: usize, internal: bool) -> usize {
    if internal && target != source && m.chain(target).any(|a| a == source) {
        return source;
    }
    m.chain(source)
        .skip(1)
        .find(|&a| a != target && m.chain(target).any(|t| t == a))
        .unwrap_or(0)
}

/// A waiting state (section 5, Kinds of state): atomic, no `invoke`, not a router, and no
/// `done` on itself or an ancestor.
fn is_waiting(m: &LoadedMachine, i: usize) -> bool {
    let node = &m.nodes[i];
    !m.is_compound(i)
        && !node.is_final
        && node.invoke.is_none()
        && node.router.is_none()
        && !m.handles(i, "done")
}

/// Whether to draw ` (implicit)` `error` to `failed`: a non-final atomic state with an
/// `invoke`, an `onentry` or a `timeout_s`, where nothing in its chain handles `error`.
fn implicit_error(m: &LoadedMachine, i: usize) -> bool {
    let node = &m.nodes[i];
    !m.is_compound(i)
        && !node.is_final
        && (node.invoke.is_some() || !node.onentry.is_empty() || node.timeout_s.is_some())
        && !m.handles(i, "error")
}

fn script_lines(out: &mut String, key: &str, scripts: &[String]) {
    if !scripts.is_empty() {
        push(out, 2, &format!("{key}: {}", scripts.join(", ")));
    }
}

/// Mermaid entity codes for the characters that break a label.
fn escape(label: &str) -> String {
    label.replace('<', "#lt;").replace('>', "#gt;")
}

fn push(out: &mut String, level: usize, line: &str) {
    for _ in 0..level {
        out.push_str("    ");
    }
    out.push_str(line);
    out.push('\n');
}

/// The system `flowchart LR`, every line ending in `\n`.
pub fn flowchart(machines: &BTreeMap<String, LoadedMachine>, crons: &[(String, String)]) -> String {
    let mut out = String::from("flowchart LR\n");
    for id in machines.keys() {
        push(&mut out, 1, &format!("{id}[\"{id}\"]"));
    }
    for (stem, _) in crons {
        push(
            &mut out,
            1,
            &format!("{}[/\"cron: {stem}\"/]", cron_node(stem)),
        );
    }
    for (stem, machine) in crons {
        push(
            &mut out,
            1,
            &format!("{} -->|cron| {machine}", cron_node(stem)),
        );
    }
    let emits: BTreeSet<(&str, &str)> = machines
        .iter()
        .flat_map(|(id, m)| {
            m.nodes
                .iter()
                .flat_map(move |n| n.emits.iter().map(move |t| (id.as_str(), t.as_str())))
        })
        .collect();
    for (from, to) in emits {
        push(&mut out, 1, &format!("{from} -->|emits| {to}"));
    }
    out
}

/// `cron__<stem>`, with every character outside `[a-z0-9_]` replaced by `_`.
fn cron_node(stem: &str) -> String {
    let id: String = stem
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' => c,
            _ => '_',
        })
        .collect();
    format!("cron__{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{flatten, parse_machine};
    use std::path::{Path, PathBuf};

    fn load(id: &str, yaml: &str) -> LoadedMachine {
        flatten(id, PathBuf::from(id), parse_machine(yaml).unwrap())
    }

    fn fixture(rel: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
    }

    #[test]
    fn feature_matches_fixture() {
        let m = load("feature", &fixture("tests/fixtures/machines/feature.yml"));
        assert_eq!(
            machine_document(&m).unwrap(),
            fixture("tests/fixtures/graph/feature.md")
        );
    }

    #[test]
    fn escapes_angle_brackets() {
        assert_eq!(escape("a < b > c"), "a #lt; b #gt; c");
    }

    #[test]
    fn cron_ids_keep_only_lowercase_digits_and_underscore() {
        assert_eq!(cron_node("Nightly-audit.v2_x"), "cron___ightly_audit_v2_x");
    }

    #[test]
    fn internal_transition_is_drawn_inside_its_source() {
        let m = load(
            "m",
            "name: m\ndescription: d\ninitial: p\nstates:\n  p:\n    initial: a\n    transitions:\n      again: { target: b, type: internal }\n      done.state.p: done\n    states:\n      a:\n        invoke: a\n        transitions: { done: b }\n      b:\n        invoke: b\n        transitions: { done: end }\n      end: { final: true }\n  done: { final: true }\n  failed: { final: true }\n",
        );
        assert_eq!(
            state_diagram(&m).unwrap(),
            "stateDiagram-v2\n    [*] --> p\n    state p {\n        [*] --> a\n        a --> b: done\n        b --> end: done\n        p --> b: again (internal)\n        end --> [*]\n    }\n    a --> failed: error (implicit)\n    b --> failed: error (implicit)\n    p --> done: done.state.p\n    done --> [*]\n    failed --> [*]\n"
        );
    }

    #[test]
    fn handled_error_draws_no_implicit_edge_and_lists_join_scripts() {
        let m = load(
            "m",
            "name: m\ndescription: d\nonentry: [a, b]\ninitial: s\nstates:\n  s:\n    invoke: s\n    onexit: [x, y]\n    transitions: { done: done, error: failed }\n  done: { final: true }\n  failed: { final: true }\n",
        );
        assert_eq!(
            state_diagram(&m).unwrap(),
            "stateDiagram-v2\n    [*] --> s\n    s --> done: done\n    s --> failed: error\n    done --> [*]\n    failed --> [*]\n    note left of s\n        machine onentry: a, b\n    end note\n    note right of s\n        onexit: x, y\n    end note\n"
        );
    }

    #[test]
    fn unknown_target_is_an_error() {
        let m = load(
            "m",
            "name: m\ndescription: d\ninitial: s\nstates:\n  s:\n    invoke: s\n    transitions: { done: nowhere }\n  failed: { final: true }\n",
        );
        let err = state_diagram(&m).unwrap_err();
        assert!(
            err.contains("machines/m.yml: s: transition `done` targets unknown state `nowhere`"),
            "{err}"
        );
    }

    #[test]
    fn system_flowchart_orders_nodes_cron_and_distinct_emits() {
        let a = load(
            "a",
            "name: a\ndescription: d\ninitial: s\nstates:\n  s:\n    invoke: s\n    emits: [b, b]\n    transitions: { done: done }\n  done: { final: true, emits: [a] }\n  failed: { final: true }\n",
        );
        let b = load(
            "b",
            "name: b\ndescription: d\ninitial: done\nstates:\n  done: { final: true }\n  failed: { final: true }\n",
        );
        let machines = BTreeMap::from([("b".to_string(), b), ("a".to_string(), a)]);
        let crons = vec![("every-hour".to_string(), "b".to_string())];
        assert_eq!(
            flowchart(&machines, &crons),
            "flowchart LR\n    a[\"a\"]\n    b[\"b\"]\n    cron__every_hour[/\"cron: every-hour\"/]\n    cron__every_hour -->|cron| b\n    a -->|emits| a\n    a -->|emits| b\n"
        );
    }
}
