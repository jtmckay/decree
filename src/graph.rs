//! `decree graph`: Mermaid diagrams drawn from the machine arena (docs/reference/graph.md), so the
//! picture cannot drift from what the interpreter runs. Output is byte-for-byte
//! deterministic; decree renders no images.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::machine::{ChooseKind, Invoke, LoadedMachine, FAILED, MACHINES_DIR, ROUTER_MACHINE};

/// The file `decree graph` writes for the whole system, beside one per machine.
pub const SYSTEM_FILE: &str = "system.md";

/// The Markdown document for one machine, `graph/<machine>.md`: heading, description, a
/// link back to its YAML, and a `stateDiagram-v2`.
pub fn machine_document(m: &LoadedMachine) -> Result<String, String> {
    let link = format!(
        "Machine: [{MACHINES_DIR}/{id}.yml](../{MACHINES_DIR}/{id}.yml)\n\n",
        id = m.id
    );
    Ok(document(&m.id, m.description(), &link, &state_diagram(m)?))
}

/// The Markdown document for the whole system, `graph/system.md`: a link to each machine's
/// document, then a `flowchart LR` of every machine, the `emits` and `invokes` edges between
/// them, and one node per cron file. `crons` is `(file stem, machine)` in filename order.
pub fn system_document(
    machines: &BTreeMap<String, LoadedMachine>,
    crons: &[(String, String)],
) -> String {
    let list: String = machines
        .iter()
        .map(|(id, m)| format!("- [{id}]({id}.md): {}\n", m.description()))
        .collect();
    document(
        "All machines",
        "Every machine, the `emits` and `invokes` edges between them, and cron entry points.",
        &format!("{list}\n"),
        &flowchart(machines, crons),
    )
}

fn document(heading: &str, description: &str, before: &str, diagram: &str) -> String {
    format!("# {heading}\n\n{description}\n\n{before}```mermaid\n{diagram}```\n")
}

/// One edge to draw, already placed in its container.
struct Line {
    source: String,
    event: String,
    target: String,
    label: String,
}

/// The `stateDiagram-v2` for one machine, every line ending in `\n`.
fn state_diagram(m: &LoadedMachine) -> Result<String, String> {
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
            if let Some(invoke) = &node.invoke {
                label.push_str(&invoke_suffix(invoke, edge.event == "error"));
            }
            if edge.internal {
                label.push_str(" (internal)");
            }
            edges
                .entry(m.transition_domain(i, target, edge.internal))
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
    notes(m, &mut out);
    Ok(out)
}

/// The label suffix for a transition of a state with this invoke (docs/reference/graph.md).
fn invoke_suffix(invoke: &Invoke, is_error: bool) -> String {
    match invoke {
        Invoke::Script(_) => String::new(),
        Invoke::Check(_) => " (check)".into(),
        _ if is_error => String::new(),
        Invoke::Choose(c) => match (c.choose, &c.router) {
            (ChooseKind::Model, Some(router)) => format!(" (model: {router})"),
            (ChooseKind::Model, None) => " (model)".into(),
            (ChooseKind::Person, _) => " (person)".into(),
        },
        Invoke::Machine(mi) => format!(" (machine: {})", mi.machine),
    }
}

/// The notes after the root's last line: the root's scripts, then each state's decision,
/// child machine and scripts, in arena order.
fn notes(m: &LoadedMachine, out: &mut String) {
    let root = m.root();
    if !root.onentry.is_empty() || !root.onexit.is_empty() {
        let initial = root.initial.as_deref().unwrap_or_default();
        push(out, 1, &format!("note left of {initial}"));
        script_lines(out, "machine onentry", &root.onentry);
        script_lines(out, "machine onexit", &root.onexit);
        push(out, 1, "end note");
    }
    // Arena order: depth-first, children in `BTreeMap` order.
    for node in m.nodes.iter().skip(1) {
        let mut lines = Vec::new();
        match &node.invoke {
            Some(Invoke::Check(c)) => lines.push(format!("check: {}", c.check)),
            Some(Invoke::Choose(c)) if c.choose == ChooseKind::Model => {
                let router = c.router.as_deref().unwrap_or(ROUTER_MACHINE);
                let mut line = format!("model: {router}");
                if let Some(n) = c.min_confidence {
                    let _ = write!(line, ", min_confidence {n}");
                }
                lines.push(line);
            }
            Some(Invoke::Machine(mi)) => lines.push(format!("machine: {}", mi.machine)),
            Some(Invoke::Choose(c)) => {
                lines.push(format!("person: {}", c.ask.as_deref().unwrap_or_default()))
            }
            Some(Invoke::Script(_)) | None => {}
        }
        if !node.onentry.is_empty() {
            lines.push(format!("onentry: {}", node.onentry.join(", ")));
        }
        if !node.onexit.is_empty() {
            lines.push(format!("onexit: {}", node.onexit.join(", ")));
        }
        if !lines.is_empty() {
            push(out, 1, &format!("note right of {}", node.id));
            for line in &lines {
                push(out, 2, line);
            }
            push(out, 1, "end note");
        }
    }
}

/// docs/reference/graph.md steps 1–4 for container `c` (the root or a compound state) at `level`.
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

/// Whether to draw ` (implicit)` `error` to `failed`: a non-final atomic state that
/// invokes a script, a machine, `choose: model` or `choose: person`, or has an `onentry`,
/// where nothing in its chain handles `error`. A `check` cannot fail.
fn implicit_error(m: &LoadedMachine, i: usize) -> bool {
    let node = &m.nodes[i];
    let can_fail = match &node.invoke {
        Some(Invoke::Check(_)) | None => !node.onentry.is_empty(),
        Some(_) => true,
    };
    !m.is_compound(i) && !node.is_final && can_fail && !m.handles(i, "error")
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
fn flowchart(machines: &BTreeMap<String, LoadedMachine>, crons: &[(String, String)]) -> String {
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
    let invokes: BTreeSet<(&str, &str)> = machines
        .iter()
        .flat_map(|(id, m)| {
            m.invoked_machines()
                .into_iter()
                .map(move |(_, child)| (id.as_str(), child))
        })
        .collect();
    for (from, to) in invokes {
        push(&mut out, 1, &format!("{from} -->|invokes| {to}"));
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
    use crate::machine::load_machine_text;
    use std::path::Path;

    fn load(id: &str, yaml: &str) -> LoadedMachine {
        load_machine_text(id, yaml).unwrap()
    }

    fn fixture(rel: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
    }

    #[test]
    fn feature_matches_fixture() {
        let m = load("feature", &fixture("mock/.decree/machines/feature.yml"));
        assert_eq!(
            machine_document(&m).unwrap(),
            fixture("mock/.decree/graph/feature.md")
        );
    }

    #[test]
    fn decision_labels_and_notes() {
        let m = load(
            "m",
            "name: m\ndescription: d\ninitial: a\nstates:\n  a:\n    invoke: { check: { matches: '<ok>' }, input: s }\n    transitions: { yes: b, no: s }\n  s:\n    invoke: s\n    transitions: { done: a }\n  b:\n    invoke: { choose: model, question: \"Q?\", router: picker }\n    transitions:\n      go: { target: c, description: Go. }\n      stop: { target: done, description: Stop. }\n      error: failed\n  c:\n    invoke: { machine: child }\n    onentry: [prep]\n    transitions: { done: done, error: failed }\n  done: { final: true }\n  failed: { final: true }\n",
        );
        assert_eq!(
            state_diagram(&m).unwrap(),
            "stateDiagram-v2\n    [*] --> a\n    a --> s: no (check)\n    a --> b: yes (check)\n    b --> failed: error\n    b --> c: go (model: picker)\n    b --> done: stop (model: picker)\n    c --> done: done (machine: child)\n    c --> failed: error\n    s --> a: done\n    s --> failed: error (implicit)\n    done --> [*]\n    failed --> [*]\n    note right of a\n        check: matches '<ok>'\n    end note\n    note right of b\n        model: picker\n    end note\n    note right of c\n        machine: child\n        onentry: prep\n    end note\n"
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
            "name: a\ndescription: d\ninitial: s\nstates:\n  s:\n    invoke: s\n    emits: [b, b]\n    transitions: { done: t }\n  t:\n    invoke: { machine: b }\n    transitions: { done: done }\n  done: { final: true, emits: [a] }\n  failed: { final: true }\n",
        );
        let b = load(
            "b",
            "name: b\ndescription: d\ninitial: done\nstates:\n  done: { final: true }\n  failed: { final: true }\n",
        );
        let machines = BTreeMap::from([("b".to_string(), b), ("a".to_string(), a)]);
        let crons = vec![("every-hour".to_string(), "b".to_string())];
        assert_eq!(
            flowchart(&machines, &crons),
            "flowchart LR\n    a[\"a\"]\n    b[\"b\"]\n    cron__every_hour[/\"cron: every-hour\"/]\n    cron__every_hour -->|cron| b\n    a -->|emits| a\n    a -->|emits| b\n    a -->|invokes| b\n"
        );
    }
}
