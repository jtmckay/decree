use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::DecreeError;

/// Default context window size in tokens.
pub const DEFAULT_CTX_SIZE: u32 = 4096;

/// GGUF model download URL.
const GGUF_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q5_k_m.gguf";

/// Statistics from a generation run.
pub struct GenerationStats {
    pub tokens_generated: usize,
    pub prefill_time: std::time::Duration,
    pub generation_time: std::time::Duration,
}

/// A chat message with role and content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Initialize the llama backend. If `suppress_logs` is true, calls
/// `void_logs()` to silence llama.cpp internal diagnostics.
pub fn init_backend(suppress_logs: bool) -> Result<LlamaBackend, DecreeError> {
    let mut backend =
        LlamaBackend::init().map_err(|e| DecreeError::Model(format!("backend init: {e}")))?;
    if suppress_logs {
        backend.void_logs();
    }
    Ok(backend)
}

/// Load the GGUF model from the given path.
pub fn load_model(
    backend: &LlamaBackend,
    path: &Path,
    n_gpu_layers: u32,
) -> Result<LlamaModel, DecreeError> {
    let model_params = if n_gpu_layers > 0 {
        LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers)
    } else {
        LlamaModelParams::default()
    };
    LlamaModel::load_from_file(backend, path, &model_params)
        .map_err(|e| DecreeError::Model(format!("load model: {e}")))
}

/// Create an inference context with the given context size.
pub fn create_context<'a>(
    model: &'a LlamaModel,
    backend: &LlamaBackend,
    ctx_size: u32,
) -> Result<LlamaContext<'a>, DecreeError> {
    let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(ctx_size));
    model
        .new_context(backend, ctx_params)
        .map_err(|e| DecreeError::Model(format!("create context: {e}")))
}

/// Tokenize text using the model's tokenizer.
pub fn tokenize(
    model: &LlamaModel,
    text: &str,
    add_bos: bool,
) -> Result<Vec<LlamaToken>, DecreeError> {
    let bos = if add_bos {
        AddBos::Always
    } else {
        AddBos::Never
    };
    model
        .str_to_token(text, bos)
        .map_err(|e| DecreeError::Model(format!("tokenize: {e}")))
}

/// Count the number of tokens in the given text.
pub fn count_tokens(model: &LlamaModel, text: &str) -> Result<usize, DecreeError> {
    tokenize(model, text, false).map(|t| t.len())
}

/// Decode a single token to its string representation using `token_to_piece`.
/// Uses a stateful decoder for proper multi-byte UTF-8 handling.
fn decode_token(
    model: &LlamaModel,
    token: LlamaToken,
    decoder: &mut encoding_rs::Decoder,
) -> Result<String, DecreeError> {
    model
        .token_to_piece(token, decoder, false, None)
        .map_err(|e| DecreeError::Model(format!("token_to_piece: {e}")))
}

/// Build a ChatML-formatted prompt string from a list of messages.
/// If `add_generation_prompt` is true, appends `<|im_start|>assistant\n` at the end.
pub fn build_chatml(messages: &[ChatMessage], add_generation_prompt: bool) -> String {
    let mut prompt = String::new();
    for msg in messages {
        prompt.push_str("<|im_start|>");
        prompt.push_str(&msg.role);
        prompt.push('\n');
        prompt.push_str(&msg.content);
        prompt.push_str("<|im_end|>\n");
    }
    if add_generation_prompt {
        prompt.push_str("<|im_start|>assistant\n");
    }
    prompt
}

/// Generate text from the model given tokenized input.
///
/// Returns the generated text and timing statistics.
/// Calls `on_token` for each generated token piece (for streaming output).
pub fn generate(
    ctx: &mut LlamaContext,
    model: &LlamaModel,
    tokens: &[LlamaToken],
    max_tokens: u32,
    mut on_token: impl FnMut(&str),
) -> Result<(String, GenerationStats), DecreeError> {
    let n_tokens = tokens.len();

    // Prefill: feed all prompt tokens
    let prefill_start = Instant::now();
    let mut batch = LlamaBatch::new(n_tokens + max_tokens as usize, 1);
    for (i, &token) in tokens.iter().enumerate() {
        let is_last = i == n_tokens - 1;
        batch
            .add(token, i as i32, &[0], is_last)
            .map_err(|e| DecreeError::Model(format!("batch add: {e}")))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| DecreeError::Model(format!("prefill decode: {e}")))?;
    let prefill_time = prefill_start.elapsed();

    // Generation loop
    let gen_start = Instant::now();
    let mut output = String::new();
    let mut n_generated: usize = 0;
    let mut pos = n_tokens;

    let eos = model.token_eos();

    // Create a UTF-8 decoder for token-to-string conversion
    let mut decoder = encoding_rs::UTF_8.new_decoder();

    // Set up sampler: temperature + top-p + dist
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.7),
        LlamaSampler::top_p(0.9, 1),
        LlamaSampler::dist(42),
    ]);

    for _ in 0..max_tokens {
        let new_token = sampler.sample(ctx, batch.n_tokens() - 1);
        sampler.accept(new_token);

        if new_token == eos {
            break;
        }

        let piece = decode_token(model, new_token, &mut decoder)?;
        on_token(&piece);
        output.push_str(&piece);
        n_generated += 1;

        // Feed new token back
        batch.clear();
        batch
            .add(new_token, pos as i32, &[0], true)
            .map_err(|e| DecreeError::Model(format!("batch add gen: {e}")))?;
        ctx.decode(&mut batch)
            .map_err(|e| DecreeError::Model(format!("gen decode: {e}")))?;
        pos += 1;
    }

    let generation_time = gen_start.elapsed();

    Ok((
        output,
        GenerationStats {
            tokens_generated: n_generated,
            prefill_time,
            generation_time,
        },
    ))
}

/// Detect the compile-time build backend.
pub fn detect_build_backend() -> &'static str {
    if cfg!(feature = "vulkan") {
        "Vulkan"
    } else if cfg!(feature = "cuda") {
        "CUDA"
    } else if cfg!(feature = "metal") {
        "Metal"
    } else {
        "CPU"
    }
}

/// Detect the GPU device name, or a "none" message if CPU-only.
pub fn detect_gpu(n_gpu_layers: u32) -> String {
    if cfg!(feature = "vulkan") || cfg!(feature = "cuda") || cfg!(feature = "metal") {
        if n_gpu_layers == 0 {
            "none (n_gpu_layers = 0)".to_string()
        } else {
            format!(
                "{} device (n_gpu_layers = {})",
                detect_build_backend(),
                n_gpu_layers
            )
        }
    } else {
        "none (CPU-only build)".to_string()
    }
}

/// Ensure the GGUF model file is available. If missing, offer to download it.
/// Returns the resolved path to the model file.
pub fn ensure_model(config: &Config) -> Result<PathBuf, DecreeError> {
    let resolved = config.resolved_model_path();
    if resolved.exists() {
        return Ok(resolved);
    }

    eprintln!("Model not found at: {}", resolved.display());

    // Check if stdin is a TTY for interactive prompt
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let confirm =
            inquire::Confirm::new("Download Qwen 2.5 1.5B-Instruct Q5_K_M (~1.1 GB)?")
                .with_default(true)
                .prompt()
                .unwrap_or(false);

        if confirm {
            download_model(&resolved)?;
            return Ok(resolved);
        }
    }

    eprintln!("You can download the model manually from:");
    eprintln!("  {GGUF_URL}");
    eprintln!("Place it at: {}", resolved.display());
    Err(DecreeError::Model("model file not found".to_string()))
}

/// Download the model file with a progress bar.
fn download_model(dest: &Path) -> Result<(), DecreeError> {
    use std::io::Write;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    eprintln!("Downloading model from:");
    eprintln!("  {GGUF_URL}");

    let response = reqwest::blocking::get(GGUF_URL)
        .map_err(|e| DecreeError::Model(format!("download failed: {e}")))?;

    if !response.status().is_success() {
        return Err(DecreeError::Model(format!(
            "download failed: HTTP {}",
            response.status()
        )));
    }

    let total_size = response.content_length().unwrap_or(0);
    let pb = indicatif::ProgressBar::new(total_size);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .expect("valid template")
            .progress_chars("#>-"),
    );

    let tmp_path = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp_path)?;
    let mut downloaded: u64 = 0;

    let mut reader = response;
    let mut buf = [0u8; 8192];
    loop {
        use std::io::Read;
        let n = reader
            .read(&mut buf)
            .map_err(|e| DecreeError::Model(format!("download read: {e}")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("download complete");
    drop(file);
    std::fs::rename(&tmp_path, dest)?;
    eprintln!("Model saved to: {}", dest.display());
    Ok(())
}
