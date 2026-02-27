use std::io::{self, BufRead, IsTerminal, Read, Write};

use crate::config::Config;
use crate::error::{find_project_root, DecreeError};
use crate::llm::{self, ChatMessage, DEFAULT_CTX_SIZE};
use crate::session::Session;

/// Run the `decree ai` command.
///
/// Modes:
/// - Interactive REPL: no -p, stdin is TTY
/// - One-shot: -p "prompt" provided
/// - Piped: stdin is not TTY
pub fn run(
    prompt: Option<&str>,
    json: bool,
    max_tokens: Option<u32>,
    resume: Option<Option<&str>>,
) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;
    let max_gen = max_tokens.unwrap_or(DEFAULT_CTX_SIZE);
    let ctx_size = DEFAULT_CTX_SIZE;

    // Ensure model is available
    let model_path = llm::ensure_model(&config)?;

    // Initialize backend (suppress logs for ai command)
    let backend = llm::init_backend(true)?;
    let model = llm::load_model(&backend, &model_path, config.ai.n_gpu_layers)?;
    let mut ctx = llm::create_context(&model, &backend, ctx_size)?;

    let stdin_is_tty = io::stdin().is_terminal();
    let has_piped_input = !stdin_is_tty;

    // Determine mode
    match (prompt, has_piped_input, resume) {
        // Resume session (works regardless of stdin TTY state)
        (None, _, Some(session_id)) => {
            let session = match session_id {
                Some(id) => Session::load(&root, id)?,
                None => Session::load_latest(&root)?,
            };
            run_repl(&root, &model, &mut ctx, json, max_gen, ctx_size, Some(session))
        }

        // Piped input with -p: -p is system prompt, stdin is user content
        (Some(sys_prompt), true, _) => {
            let mut stdin_content = String::new();
            io::stdin()
                .read_to_string(&mut stdin_content)
                .map_err(|e| DecreeError::Model(format!("read stdin: {e}")))?;
            let stdin_content = stdin_content.trim().to_string();

            run_oneshot(
                &model,
                &mut ctx,
                Some(sys_prompt),
                &stdin_content,
                json,
                max_gen,
            )
        }

        // Piped input without -p: stdin is the prompt (no REPL, no session)
        (None, true, None) => {
            let mut stdin_content = String::new();
            io::stdin()
                .read_to_string(&mut stdin_content)
                .map_err(|e| DecreeError::Model(format!("read stdin: {e}")))?;
            let stdin_content = stdin_content.trim().to_string();

            run_oneshot(&model, &mut ctx, None, &stdin_content, json, max_gen)
        }

        // One-shot: -p "prompt"
        (Some(p), false, _) => run_oneshot(&model, &mut ctx, None, p, json, max_gen),

        // Interactive REPL: no -p, TTY stdin, no resume
        (None, false, None) => {
            run_repl(&root, &model, &mut ctx, json, max_gen, ctx_size, None)
        }
    }
}

/// Run a single prompt and print the response. No session file created.
fn run_oneshot(
    model: &llama_cpp_2::model::LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext,
    system_prompt: Option<&str>,
    user_content: &str,
    json: bool,
    max_tokens: u32,
) -> Result<(), DecreeError> {
    let mut messages = Vec::new();

    // System prompt for JSON mode
    let sys_content = if json {
        match system_prompt {
            Some(sp) => format!("{sp}\n\nYou must respond with valid JSON only."),
            None => "You must respond with valid JSON only.".to_string(),
        }
    } else {
        system_prompt.unwrap_or("You are a helpful assistant.").to_string()
    };

    messages.push(ChatMessage {
        role: "system".to_string(),
        content: sys_content,
    });

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content.to_string(),
    });

    let prompt_text = llm::build_chatml(&messages, true);
    let tokens = llm::tokenize(model, &prompt_text, false)?;

    ctx.clear_kv_cache();

    let (output, _stats) = llm::generate(ctx, model, &tokens, max_tokens, |piece| {
        print!("{piece}");
        let _ = io::stdout().flush();
    })?;

    // Ensure trailing newline
    if !output.ends_with('\n') {
        println!();
    }

    Ok(())
}

/// Run the interactive REPL with session persistence.
fn run_repl(
    root: &std::path::Path,
    model: &llama_cpp_2::model::LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext,
    json: bool,
    max_gen: u32,
    ctx_size: u32,
    existing_session: Option<Session>,
) -> Result<(), DecreeError> {
    let mut session = match existing_session {
        Some(s) => {
            println!("resuming session: {}", s.id);
            s
        }
        None => {
            let s = Session::new();
            println!("decree ai — interactive mode (type 'exit' or Ctrl-D to quit)");
            println!("session: {}", s.id);
            s
        }
    };

    // Build in-memory working history from session (may be truncated for context)
    let mut working_history: Vec<ChatMessage> = session.history.clone();

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        // Calculate context usage percentage
        let pct = context_usage_pct(model, &working_history, ctx_size)?;
        print!("[{pct}%] > ");
        io::stdout().flush().map_err(DecreeError::Io)?;

        // Read user input
        let mut input = String::new();
        let bytes = reader.read_line(&mut input).map_err(DecreeError::Io)?;
        if bytes == 0 {
            // EOF (Ctrl-D)
            println!();
            break;
        }

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }
        if input == "exit" || input == "quit" {
            break;
        }

        // Append user message to both full history and working history
        let user_msg = ChatMessage {
            role: "user".to_string(),
            content: input,
        };
        session.history.push(user_msg.clone());
        working_history.push(user_msg);

        // Truncate working history if needed
        let dropped = truncate_history(model, &mut working_history, ctx_size, max_gen)?;
        if dropped > 0 {
            println!(
                "~ context: dropped {} earliest messages (history exceeded context window)",
                dropped
            );
        }

        // Build prompt
        let mut messages = Vec::new();

        // System message
        let sys = if json {
            "You are a helpful assistant. You must respond with valid JSON only."
        } else {
            "You are a helpful assistant."
        };
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: sys.to_string(),
        });
        messages.extend(working_history.iter().cloned());

        let prompt_text = llm::build_chatml(&messages, true);
        let tokens = llm::tokenize(model, &prompt_text, false)?;

        // Clear KV cache before each turn
        ctx.clear_kv_cache();

        // Generate response
        let (output, _stats) = llm::generate(ctx, model, &tokens, max_gen, |piece| {
            print!("{piece}");
            let _ = io::stdout().flush();
        })?;

        // Ensure trailing newline
        if !output.ends_with('\n') {
            println!();
        }
        println!();

        // Append assistant response
        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: output,
        };
        session.history.push(assistant_msg.clone());
        working_history.push(assistant_msg);

        // Save session to disk
        session.save(root)?;
    }

    Ok(())
}

/// Calculate the context usage percentage of the current history.
pub fn context_usage_pct(
    model: &llama_cpp_2::model::LlamaModel,
    history: &[ChatMessage],
    ctx_size: u32,
) -> Result<u32, DecreeError> {
    if history.is_empty() {
        return Ok(0);
    }
    let prompt = llm::build_chatml(history, false);
    let token_count = llm::count_tokens(model, &prompt)?;
    let pct = ((token_count as f64 / ctx_size as f64) * 100.0).round() as u32;
    Ok(pct.min(100))
}

/// Truncate the oldest messages from working history to fit within the context budget.
/// Returns the number of messages dropped.
pub fn truncate_history(
    model: &llama_cpp_2::model::LlamaModel,
    history: &mut Vec<ChatMessage>,
    ctx_size: u32,
    max_gen: u32,
) -> Result<usize, DecreeError> {
    let budget = ctx_size.saturating_sub(max_gen) as usize;
    let mut dropped: usize = 0;

    loop {
        // Build prompt with system message + current history
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: "You are a helpful assistant.".to_string(),
        }];
        messages.extend(history.iter().cloned());
        let prompt = llm::build_chatml(&messages, true);
        let token_count = llm::count_tokens(model, &prompt)?;

        if token_count <= budget {
            break;
        }

        // If history is too small to truncate further, warn and break
        if history.len() < 2 {
            eprintln!("warning: input exceeds context window, truncating");
            break;
        }

        // Drop the oldest user+assistant pair
        history.remove(0);
        dropped += 1;
        if !history.is_empty() && history[0].role == "assistant" {
            history.remove(0);
            dropped += 1;
        }
    }

    Ok(dropped)
}
