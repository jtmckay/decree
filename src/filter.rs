//! Which machines a `process` or `daemon` runs (docs/reference/cli.md, Several processes;
//! docs/decisions.md, D69), as Celery's `--queues` and `--exclude-queues` pick a worker's
//! queues: `--machine` or `--exclude-machine`, else `DECREE_MACHINES` or
//! `DECREE_EXCLUDE_MACHINES` from decree's own environment, else from `.decree/.env`. A run
//! belongs to the machine of its root run: a child run goes with its parent, whatever its
//! machine.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::error::DecreeError;
use crate::layout::MESSAGE_FILE;
use crate::message::{is_valid_id, Message};

/// The allow-list variable, as `--machine`.
pub const MACHINES_VAR: &str = "DECREE_MACHINES";
/// The deny-list variable, as `--exclude-machine`.
pub const EXCLUDE_MACHINES_VAR: &str = "DECREE_EXCLUDE_MACHINES";

/// Whether `key` is one of the two variables decree reads itself from `.decree/.env`.
pub fn is_filter_var(key: &str) -> bool {
    key == MACHINES_VAR || key == EXCLUDE_MACHINES_VAR
}

/// `--machine` and `--exclude-machine` as given, each value one comma-separated list.
#[derive(Debug, Clone, Default)]
pub struct Select {
    pub machines: Vec<String>,
    pub exclude: Vec<String>,
}

impl Select {
    pub fn new(machines: Vec<String>, exclude: Vec<String>) -> Select {
        Select { machines, exclude }
    }

    /// The filter these flags give, else the variables (`MachineFilter::resolve`), every
    /// name checked against `known` (`MachineFilter::check`).
    pub fn filter(
        &self,
        dotenv: &[(String, String)],
        known: impl Fn(&str) -> bool,
    ) -> Result<MachineFilter, DecreeError> {
        let (filter, source) = MachineFilter::resolve(
            &self.machines,
            &self.exclude,
            crate::dotenv::process_var,
            dotenv,
        )?;
        filter.check(known, &source)?;
        Ok(filter)
    }

    /// Whether a flag was given, so the variables are not read.
    pub fn is_given(&self) -> bool {
        !self.machines.is_empty() || !self.exclude.is_empty()
    }
}

/// Looks up a variable: decree's own environment, or `.decree/.env`.
type Lookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// The machines a process runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MachineFilter {
    /// Every machine: no flag and no variable.
    #[default]
    All,
    /// Only these: `--machine` or `DECREE_MACHINES`.
    Only(BTreeSet<String>),
    /// Every machine but these: `--exclude-machine` or `DECREE_EXCLUDE_MACHINES`.
    Except(BTreeSet<String>),
}

impl MachineFilter {
    /// Whether this process runs work for `machine`. Work whose machine cannot be read (a
    /// message that does not parse, a reply to no run) goes to a process that runs every
    /// machine, or all but some; a process given a list of machines leaves it.
    pub fn runs(&self, machine: Option<&str>) -> bool {
        match (self, machine) {
            (MachineFilter::All, _) => true,
            (MachineFilter::Only(names), Some(m)) => names.contains(m),
            (MachineFilter::Only(_), None) => false,
            (MachineFilter::Except(names), m) => !m.is_some_and(|m| names.contains(m)),
        }
    }

    pub fn is_all(&self) -> bool {
        matches!(self, MachineFilter::All)
    }

    /// The filter from the flags, else decree's own environment (`shell`), else
    /// `.decree/.env` (`dotenv`, its resolved variables), each list comma-separated; a flag
    /// beats both, the shell beats `.env`. Returns it with where it came from, for messages.
    /// Both lists from one place is a usage error.
    pub fn resolve(
        machines: &[String],
        exclude: &[String],
        shell: impl Fn(&str) -> Option<String>,
        dotenv: &[(String, String)],
    ) -> Result<(MachineFilter, String), DecreeError> {
        if !machines.is_empty() || !exclude.is_empty() {
            if !machines.is_empty() && !exclude.is_empty() {
                return Err(both("--machine", "--exclude-machine"));
            }
            let (flag, values) = if machines.is_empty() {
                ("--exclude-machine", exclude)
            } else {
                ("--machine", machines)
            };
            let names = names(values.iter().map(String::as_str));
            if names.is_empty() {
                return Err(DecreeError::Usage(format!("{flag} names no machine")));
            }
            return Ok((filter(flag == "--machine", names), flag.to_string()));
        }
        let from_dotenv = |key: &str| {
            dotenv
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        let sources: [(&Lookup, &str); 2] =
            [(&shell, "the environment"), (&from_dotenv, ".decree/.env")];
        for (lookup, place) in sources {
            // An empty variable is unset.
            let read = |key: &str| {
                let names = names(lookup(key).as_deref().into_iter());
                (!names.is_empty()).then_some(names)
            };
            match (read(MACHINES_VAR), read(EXCLUDE_MACHINES_VAR)) {
                (Some(_), Some(_)) => {
                    return Err(both(
                        &format!("{MACHINES_VAR} in {place}"),
                        &format!("{EXCLUDE_MACHINES_VAR} in {place}"),
                    ))
                }
                (Some(names), None) => {
                    return Ok((filter(true, names), format!("{MACHINES_VAR} in {place}")))
                }
                (None, Some(names)) => {
                    return Ok((
                        filter(false, names),
                        format!("{EXCLUDE_MACHINES_VAR} in {place}"),
                    ))
                }
                (None, None) => {}
            }
        }
        Ok((MachineFilter::All, String::new()))
    }

    /// Every name the filter gives must be a machine: one in `.decree/machines/` (`known`)
    /// or a built-in. `source` is where the names came from.
    pub fn check(&self, known: impl Fn(&str) -> bool, source: &str) -> Result<(), DecreeError> {
        let (MachineFilter::Only(names) | MachineFilter::Except(names)) = self else {
            return Ok(());
        };
        let unknown: Vec<String> = names
            .iter()
            .filter(|n| !known(n) && !crate::commands::init::is_builtin_machine(n))
            .map(|n| format!("`{n}`"))
            .collect();
        match unknown.len() {
            0 => Ok(()),
            1 => Err(DecreeError::Other(format!(
                "{source} names {}, which is not a machine in .decree/machines/",
                unknown[0]
            ))),
            _ => Err(DecreeError::Other(format!(
                "{source} names {}, which are not machines in .decree/machines/",
                unknown.join(", ")
            ))),
        }
    }
}

fn both(a: &str, b: &str) -> DecreeError {
    DecreeError::Usage(format!(
        "{a} and {b} cannot be used together: give the machines to run, or those not to run"
    ))
}

fn filter(only: bool, names: BTreeSet<String>) -> MachineFilter {
    if only {
        MachineFilter::Only(names)
    } else {
        MachineFilter::Except(names)
    }
}

/// The names in comma-separated `lists`, trimmed, without empty ones.
fn names<'a>(lists: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    lists
        .flat_map(|list| list.split(','))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect()
}

/// The machine run `id` belongs to: that of its root run, following `parent` up while a run
/// is a child (`trigger: invoke`). `None` when a `message.md` on the way cannot be read.
pub fn owner(runs_dir: &Path, id: &str) -> Option<String> {
    let mut id = id.to_string();
    let mut seen = HashSet::new();
    loop {
        if !is_valid_id(&id) || !seen.insert(id.clone()) {
            return None;
        }
        let message = Message::read(&runs_dir.join(&id).join(MESSAGE_FILE)).ok()?;
        match message.text("parent") {
            Some(parent) if message.text("trigger") == Some("invoke") => id = parent.to_string(),
            _ => return message.machine().map(String::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn no_shell(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn flags_take_repeated_comma_separated_lists() {
        let (f, source) =
            MachineFilter::resolve(&strings(&["a,b", " c ,"]), &[], no_shell, &[]).unwrap();
        assert_eq!(f, MachineFilter::Only(set(&["a", "b", "c"])));
        assert_eq!(source, "--machine");
        let (f, _) = MachineFilter::resolve(&[], &strings(&["a"]), no_shell, &[]).unwrap();
        assert_eq!(f, MachineFilter::Except(set(&["a"])));
    }

    #[test]
    fn both_lists_from_one_place_are_a_usage_error() {
        let both = MachineFilter::resolve(&strings(&["a"]), &strings(&["b"]), no_shell, &[]);
        assert!(matches!(both, Err(DecreeError::Usage(_))), "{both:?}");
        let dotenv = [
            (MACHINES_VAR.to_string(), "a".to_string()),
            (EXCLUDE_MACHINES_VAR.to_string(), "b".to_string()),
        ];
        let both = MachineFilter::resolve(&[], &[], no_shell, &dotenv);
        assert!(matches!(both, Err(DecreeError::Usage(_))), "{both:?}");
        let empty = MachineFilter::resolve(&strings(&[","]), &[], no_shell, &[]);
        assert!(matches!(empty, Err(DecreeError::Usage(_))), "{empty:?}");
    }

    #[test]
    fn a_flag_beats_the_shell_which_beats_dotenv() {
        let dotenv = [(MACHINES_VAR.to_string(), "a".to_string())];
        let shell = |k: &str| (k == EXCLUDE_MACHINES_VAR).then(|| "b".to_string());
        let (f, source) = MachineFilter::resolve(&[], &[], shell, &dotenv).unwrap();
        assert_eq!(f, MachineFilter::Except(set(&["b"])));
        assert_eq!(source, "DECREE_EXCLUDE_MACHINES in the environment");
        let (f, _) = MachineFilter::resolve(&strings(&["c"]), &[], shell, &dotenv).unwrap();
        assert_eq!(f, MachineFilter::Only(set(&["c"])));
        let (f, source) = MachineFilter::resolve(&[], &[], no_shell, &dotenv).unwrap();
        assert_eq!(f, MachineFilter::Only(set(&["a"])));
        assert_eq!(source, "DECREE_MACHINES in .decree/.env");
        // An empty variable is unset.
        let empty = |k: &str| (k == MACHINES_VAR).then(String::new);
        let (f, _) = MachineFilter::resolve(&[], &[], empty, &dotenv).unwrap();
        assert_eq!(f, MachineFilter::Only(set(&["a"])));
        let (f, _) = MachineFilter::resolve(&[], &[], no_shell, &[]).unwrap();
        assert_eq!(f, MachineFilter::All);
    }

    #[test]
    fn runs_what_it_lists() {
        let only = MachineFilter::Only(set(&["a"]));
        assert!(only.runs(Some("a")) && !only.runs(Some("b")) && !only.runs(None));
        let except = MachineFilter::Except(set(&["a"]));
        assert!(!except.runs(Some("a")) && except.runs(Some("b")) && except.runs(None));
        assert!(MachineFilter::All.runs(None));
    }

    #[test]
    fn unknown_names_are_errors_built_ins_are_not() {
        let f = MachineFilter::Only(set(&["a", "develop", "nope"]));
        let err = f.check(|n| n == "a", "--machine").unwrap_err().to_string();
        assert_eq!(
            err,
            "--machine names `nope`, which is not a machine in .decree/machines/"
        );
        assert!(MachineFilter::All.check(|_| false, "").is_ok());
    }
}
