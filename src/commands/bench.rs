use std::time::Instant;

use crate::config::Config;
use crate::error::{find_project_root, DecreeError};
use crate::llm::{self, ChatMessage, DEFAULT_CTX_SIZE};

const DEFAULT_BENCH_PROMPT: &str = "The answer to life, the universe, and everything is";
const DEFAULT_MAX_TOKENS: u32 = 200;
const DEFAULT_RUNS: u32 = 3;
const SEPARATOR: &str = "────────────────────────────────────────────────────────";

/// Run the `decree bench` command.
pub fn run(
    prompt: Option<&str>,
    runs: u32,
    max_tokens: Option<u32>,
    ctx: Option<u32>,
    verbose: bool,
) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;
    let max_gen = max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    let ctx_size = ctx.unwrap_or(DEFAULT_CTX_SIZE);
    let prompt_text = prompt.unwrap_or(DEFAULT_BENCH_PROMPT);
    let num_runs = if runs == 0 { DEFAULT_RUNS } else { runs };

    // Ensure model is available
    let model_path = llm::ensure_model(&config)?;

    // Build the ChatML prompt
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are a helpful assistant.".to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: prompt_text.to_string(),
        },
    ];
    let full_prompt = llm::build_chatml(&messages, true);

    // Truncate prompt display at ~50 chars
    let display_prompt = if prompt_text.len() > 50 {
        format!("\"{}...\"", &prompt_text[..47])
    } else {
        format!("\"{prompt_text}\"")
    };

    // Cold load: initialize backend and load model (timed for run 1)
    let init_start = Instant::now();
    let backend = llm::init_backend(!verbose)?;
    let model = llm::load_model(&backend, &model_path, config.ai.n_gpu_layers)?;
    let cold_init_time = init_start.elapsed();

    let approx_tokens = llm::count_tokens(&model, &full_prompt).unwrap_or(0);

    // Print header
    println!("{SEPARATOR}");
    println!(
        "Model:    {}",
        model_path
            .canonicalize()
            .unwrap_or(model_path.clone())
            .display()
    );
    println!("Build:    {}", llm::detect_build_backend());
    println!("GPU:      {}", llm::detect_gpu(config.ai.n_gpu_layers));
    println!("Ctx:      {ctx_size} tokens");
    println!("Prompt:   {display_prompt} (~{approx_tokens} tokens)");
    println!("Max gen:  {max_gen} tokens / run");
    println!();

    // Run table header
    println!(
        "   {:>3}   {:>7}   {:>7}   {:>7}   {:>7}",
        "run", "init", "prefill", "gen", "tok/s"
    );

    let mut run_results: Vec<RunResult> = Vec::new();
    let mut last_output = String::new();

    let tokens = llm::tokenize(&model, &full_prompt, false)?;

    for run_num in 1..=num_runs {
        // Run 1 uses the cold init time; subsequent runs reuse the model (init = 0)
        let init_time = if run_num == 1 {
            cold_init_time
        } else {
            std::time::Duration::from_secs(0)
        };

        let mut bench_ctx = llm::create_context(&model, &backend, ctx_size)?;
        bench_ctx.clear_kv_cache();

        let (output, stats) =
            llm::generate(&mut bench_ctx, &model, &tokens, max_gen, |_| {})?;

        let tok_per_sec = if stats.generation_time.as_secs_f64() > 0.0 {
            stats.tokens_generated as f64 / stats.generation_time.as_secs_f64()
        } else {
            0.0
        };

        let result = RunResult {
            init_time,
            prefill_time: stats.prefill_time,
            gen_time: stats.generation_time,
            tok_per_sec,
        };

        // Print run row
        let cold_annotation = if run_num == 1 && num_runs > 1 {
            "  <- cold (model loading included)"
        } else {
            ""
        };
        println!(
            "   {:>3}   {:>6.2}s   {:>6.2}s   {:>6.2}s   {:>7.1}{cold_annotation}",
            run_num,
            result.init_time.as_secs_f64(),
            result.prefill_time.as_secs_f64(),
            result.gen_time.as_secs_f64(),
            result.tok_per_sec,
        );

        last_output = output;
        run_results.push(result);
    }

    // Output sample
    println!();
    let display_output = if last_output.len() > 80 {
        format!("\"{}...\"", &last_output[..77])
    } else {
        format!("\"{}\"", last_output.trim())
    };
    println!("Output:   {display_output}");

    // Summary line (warm runs only, excluding run 1)
    if num_runs > 1 {
        let warm_runs: Vec<&RunResult> = run_results.iter().skip(1).collect();
        let avg_prefill = warm_runs
            .iter()
            .map(|r| r.prefill_time.as_secs_f64())
            .sum::<f64>()
            / warm_runs.len() as f64;
        let avg_tok_s =
            warm_runs.iter().map(|r| r.tok_per_sec).sum::<f64>() / warm_runs.len() as f64;

        println!();
        println!("avg prefill (warm): {avg_prefill:.2}s   avg tok/s (warm): {avg_tok_s:.1}");
    }

    println!("{SEPARATOR}");

    Ok(())
}

struct RunResult {
    init_time: std::time::Duration,
    prefill_time: std::time::Duration,
    gen_time: std::time::Duration,
    tok_per_sec: f64,
}
