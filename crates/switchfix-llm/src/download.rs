//! Download a small GGUF model into the local models directory.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

/// Known good small models (≤300 MB preferred; Qwen IQ2_M ≈ 313 MB).
pub fn catalog() -> &'static [(&'static str, &'static str)] {
    &[
        // ~409 MB Q4_0 — loads on stock llama_cpp 0.3.x. Best RU in size class.
        (
            "qwen2.5-0.5b-instruct-q4_0",
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_0.gguf",
        ),
        // ~100 MB — fastest, weak on Russian.
        (
            "smollm2-135m-instruct-q4_k_m",
            "https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/resolve/main/SmolLM2-135M-Instruct-Q4_K_M.gguf",
        ),
    ]
}

fn url_for(id: &str) -> Option<&'static str> {
    if let Ok(u) = std::env::var("SWITCHFIX_LLM_URL") {
        if !u.is_empty() {
            // Leaked for convenience of the current process only.
            return Some(Box::leak(u.into_boxed_str()));
        }
    }
    catalog()
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, url)| *url)
}

fn models_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SWITCHFIX_LLM_DIR") {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(d).join("switchfix").join("models").join("llm");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".local/share/switchfix/models")
        .join("llm")
}

pub fn pull(id: &str) -> ExitCode {
    let dir = models_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let dest = dir.join(format!("{id}.gguf"));
    if dest.is_file() {
        println!("already have {}", dest.display());
        return ExitCode::SUCCESS;
    }

    let Some(url) = url_for(id) else {
        eprintln!("unknown model id '{id}'. Known:");
        for (name, _) in catalog() {
            eprintln!("  {name}");
        }
        eprintln!("or set SWITCHFIX_LLM_URL=https://…/model.gguf");
        return ExitCode::FAILURE;
    };

    println!("downloading {id}");
    println!("  {url}");
    let resp = match ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .get(url)
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("download failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let tmp = dest.with_extension("part");
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut file = match std::fs::File::create(&tmp) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("create {}: {e}", tmp.display());
            return ExitCode::FAILURE;
        }
    };
    let mut buf = [0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = std::io::Write::write_all(&mut file, &buf[..n]) {
                    eprintln!("write: {e}");
                    return ExitCode::FAILURE;
                }
                done += n as u64;
                if let Some(t) = total {
                    let pct = done.saturating_mul(100) / t.max(1);
                    eprint!("\r  {done} / {t} ({pct}%)");
                    let _ = std::io::Write::flush(&mut std::io::stderr());
                } else {
                    eprint!("\r  {done} bytes");
                    let _ = std::io::Write::flush(&mut std::io::stderr());
                }
            }
            Err(e) => {
                eprintln!("\nread: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    eprintln!();
    drop(file);
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        eprintln!("rename: {e}");
        return ExitCode::FAILURE;
    }
    println!("saved {}", dest.display());
    ExitCode::SUCCESS
}
