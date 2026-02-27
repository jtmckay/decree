use std::path::Path;

use crate::config::Config;
use crate::error::{find_project_root, DecreeError};
use crate::message::MessageId;
use crate::pipeline::{self, ProcessResult};
use crate::spec;

/// Execute the `decree process` command: batch-process all unprocessed specs.
pub fn run() -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;

    let all_specs = spec::list_specs(&root)?;
    if all_specs.is_empty() {
        println!("no spec files found in specs/");
        return Ok(());
    }

    let processed = spec::read_processed(&root)?;
    let unprocessed: Vec<&String> = all_specs.iter().filter(|s| !processed.contains(*s)).collect();

    if unprocessed.is_empty() {
        println!("all specs are already processed");
        return Ok(());
    }

    println!("{} unprocessed spec(s) found", unprocessed.len());

    for spec_name in unprocessed {
        println!("\nprocessing spec: {spec_name}");

        let chain = MessageId::new_chain(0);
        let input_file = format!("specs/{spec_name}");

        // Derive message name from spec filename
        let msg_name = spec_name
            .strip_suffix(".spec.md")
            .unwrap_or(spec_name)
            .to_string();
        let msg_name = pipeline::to_kebab_case(&msg_name);

        // Read spec frontmatter for routine
        let spec_routine = read_spec_routine(&root, &input_file);

        // Build vars
        let vars = vec![("input_file".to_string(), input_file)];

        // Create inbox message
        let msg_path =
            pipeline::create_inbox_message(&root, &msg_name, &chain, "", &vars)?;

        // Process the chain
        let result =
            pipeline::process_chain(&root, &config, &msg_path, spec_routine.as_deref())?;

        match result {
            ProcessResult::Success => {
                println!("  spec {spec_name}: done");
            }
            ProcessResult::DeadLettered(reason) => {
                eprintln!("  spec {spec_name}: dead-lettered ({reason})");
                // Continue to the next spec
            }
        }
    }

    println!("\nprocess complete");
    Ok(())
}

/// Read the routine from a spec file's frontmatter.
fn read_spec_routine(root: &Path, input_file: &str) -> Option<String> {
    let path = root.join(input_file);
    let content = std::fs::read_to_string(path).ok()?;
    let fm = spec::parse_spec_frontmatter(&content);
    fm.routine
}
