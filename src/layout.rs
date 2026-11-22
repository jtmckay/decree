//! The `.decree/` layout (docs/reference/README.md): a project is machines, scripts and
//! messages, grouped by domain: each machine owns `machines/<name>/`, which holds its
//! `machine.yml`, `graph.md`, `scripts/`, `cron/`, `state/` and `order/`.

use std::path::{Path, PathBuf};

pub const DECREE_DIR: &str = ".decree";
/// `.decree/machines/`: one folder per machine, `machines/<name>/`.
pub const MACHINES_DIR: &str = "machines";
/// The machine's statechart, `machines/<name>/machine.yml`.
pub const MACHINE_FILE: &str = "machine.yml";
/// The machine's scripts, `machines/<name>/scripts/` (docs/reference/scripts.md, Resolution).
pub const SCRIPTS_DIR: &str = "scripts";
/// The machine's cron files, `machines/<name>/cron/*.md` (docs/reference/messages.md).
pub const CRON_DIR: &str = "cron";
/// What the machine keeps between runs, `machines/<name>/state/` (docs/reference/scripts.md,
/// State): a file named `*.git.*` there (`order.git.txt`, `swap/box1.git.sh`) is committed, any other is not.
pub const STATE_DIR: &str = "state";
/// The machine's ordered work, `machines/<name>/order/*.md`, which the `order` plugin runs.
pub const ORDER_DIR: &str = "order";
/// The machine's diagram, `machines/<name>/graph.md` (docs/reference/graph.md).
pub const GRAPH_FILE: &str = "graph.md";
/// The diagram of every machine, `.decree/system.md` (docs/reference/graph.md).
pub const SYSTEM_GRAPH_FILE: &str = "system.md";
pub const INBOX_DIR: &str = "inbox";
pub const RUNS_DIR: &str = "runs";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// `.decree/cache/`: what decree and scripts keep that git ignores, such as the record of
/// each plugin install (docs/reference/plugins.md).
pub const CACHE_DIR: &str = "cache";

/// `.decree/attention/`: one file per run that needs a person; a file with `holds: true`, or
/// one decree cannot read, stops `process` and `daemon` (docs/reference/cli.md, Attention).
pub const ATTENTION_DIR: &str = "attention";

/// `runs/<id>/hold`: written by a script (the built-in `keep`), it makes the run's
/// attention file hold the queue (docs/reference/scripts.md, Run directory).
pub const HOLD_FILE: &str = "hold";

/// `runs/<id>/leftovers`: written by a script (the built-in `clean_tree`), it names the
/// runs that may have left the changes this run found; the run's attention file quotes it
/// (docs/reference/scripts.md, Run directory).
pub const LEFTOVERS_FILE: &str = "leftovers";

/// `runs/<id>/STOP`: an agent's question, which the run's attention file quotes
/// (docs/reference/scripts.md).
pub const STOP_FILE: &str = "STOP";

/// `.decree/process.log`: what `process` and `daemon` print as runs go, kept even with
/// `--quiet`, its last 2 MiB (docs/reference/cli.md, Output).
pub const PROCESS_LOG_FILE: &str = "process.log";

/// The dotenv file every script gets, `.decree/.env` (docs/reference/scripts.md,
/// Environment). Never committed, as no `.env*` file but `ENV_EXAMPLE_FILE` is.
pub const ENV_FILE: &str = ".env";

/// The committed template of the `.env*` files, which decree never reads.
pub const ENV_EXAMPLE_FILE: &str = ".env.example";

/// The claimed message, in the run directory (docs/reference/README.md).
pub const MESSAGE_FILE: &str = "message.md";

/// Folder in the run directory that delivered replies are moved into (docs/reference/messages.md).
pub const RECEIVED_DIR: &str = "received";

/// `machines/<machine>`, relative to `.decree/`.
pub fn machine_dir_rel(machine: &str) -> String {
    format!("{MACHINES_DIR}/{machine}")
}

/// `machines/<machine>/machine.yml`, relative to `.decree/`.
pub fn machine_file_rel(machine: &str) -> String {
    format!("{MACHINES_DIR}/{machine}/{MACHINE_FILE}")
}

/// `<decree_dir>/machines/<machine>/`.
pub fn machine_dir(decree_dir: &Path, machine: &str) -> PathBuf {
    decree_dir.join(MACHINES_DIR).join(machine)
}

/// `.decree/machines/<machine>/state/`, the folder `DECREE_STATE_DIR` names for `machine`'s
/// scripts.
pub fn state_dir(project_root: &Path, machine: &str) -> PathBuf {
    machine_dir(&project_root.join(DECREE_DIR), machine).join(STATE_DIR)
}
