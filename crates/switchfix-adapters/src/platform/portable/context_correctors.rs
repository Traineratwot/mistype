//! Optional context backends: LanguageTool HTTP and external LLM process.
//!
//! Hybrid layer over the dictionary [`SpellChecker`]. Both implement
//! [`ContextCorrector`]; failures are soft (empty / `is_ok` veto only when sure).

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use switchfix_app::ports::{ContextCorrector, ContextFix, SpellError};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{debug, warn};

const TIMEOUT: Duration = Duration::from_millis(800);

// ---------------------------------------------------------------------------
// LanguageTool (HTTP)
// ---------------------------------------------------------------------------

/// LanguageTool server (`java -cp languagetool-server.jar org.languagetool.server.HTTPServer --port 8010`).
pub struct LanguageToolCorrector {
    base: String,
    client: reqwest_like::Client,
}

impl LanguageToolCorrector {
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        Self {
            base,
            client: reqwest_like::Client::new(),
        }
    }
}

#[async_trait]
impl ContextCorrector for LanguageToolCorrector {
    fn name(&self) -> &'static str {
        "languagetool"
    }

    async fn correct(
        &self,
        word: &str,
        context: &str,
        lang: &str,
    ) -> Result<Vec<ContextFix>, SpellError> {
        // `context` is the typed sentence ending with the target word.
        // Fall back to word-only when the tail is empty.
        let text = if context.trim().is_empty() {
            word.to_string()
        } else if context.trim_end().ends_with(word) {
            context.trim().to_string()
        } else {
            format!("{} {}", context.trim(), word)
        };
        let lt_lang = match lang {
            "ru" => "ru-RU",
            "en" => "en-US",
            other => other,
        };
        let body = format!(
            "text={}&language={}",
            form_urlencode(&text),
            form_urlencode(lt_lang)
        );
        let url = format!("{}/v2/check", self.base);
        let resp = self
            .client
            .post_form(&url, &body, TIMEOUT)
            .await
            .map_err(|e| SpellError::Failed(format!("languagetool: {e}")))?;

        parse_lt_response(&resp, word, &text)
    }
}

fn parse_lt_response(json: &str, word: &str, full_text: &str) -> Result<Vec<ContextFix>, SpellError> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| SpellError::Failed(format!("lt json: {e}")))?;
    let Some(matches) = v.get("matches").and_then(|m| m.as_array()) else {
        return Ok(vec![ContextFix {
            text: word.to_string(),
            confidence: 0.9,
            is_ok: true,
        }]);
    };

    // Word start offset inside `full_text` (context + word).
    let word_start = full_text
        .rfind(word)
        .unwrap_or_else(|| full_text.len().saturating_sub(word.chars().count()));
    let word_end = word_start + word.chars().count();

    let mut fixes = Vec::new();
    let mut saw_word_match = false;
    for m in matches {
        let offset = m.get("offset").and_then(|o| o.as_u64()).unwrap_or(0) as usize;
        let length = m.get("length").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
        // Only consider matches that overlap the target word.
        if offset + length <= word_start || offset >= word_end {
            continue;
        }
        saw_word_match = true;
        if let Some(reps) = m.get("replacements").and_then(|r| r.as_array()) {
            for r in reps.iter().take(3) {
                if let Some(val) = r.get("value").and_then(|x| x.as_str()) {
                    if !val.is_empty() && val != word {
                        fixes.push(ContextFix {
                            text: val.to_string(),
                            confidence: 0.88,
                            is_ok: false,
                        });
                    }
                }
            }
        }
    }

    if fixes.is_empty() && !saw_word_match {
        // LT found nothing on this word — treat as "word is fine" veto.
        return Ok(vec![ContextFix {
            text: word.to_string(),
            confidence: 0.85,
            is_ok: true,
        }]);
    }
    Ok(fixes)
}

// ---------------------------------------------------------------------------
// External LLM / script process
// ---------------------------------------------------------------------------

const COLD_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(900);

/// Persistent `cmd` subprocess. Protocol (JSON **lines**):
///
/// stdin:  `{"word":"приветт","context":"…","lang":"ru"}\n`
/// stdout: `{"suggestions":["привет"],"is_ok":false}\n` or `{"is_ok":true}\n`
///
/// The process is kept alive across words (model load is expensive). First
/// request uses a longer cold-start timeout. On crash/timeout the child is
/// restarted. One-shot tools still work: they answer one line and exit —
/// we respawn on EOF.
pub struct ProcessCorrector {
    cmd: String,
    inner: Mutex<Option<ProcHandle>>,
}

struct ProcHandle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    used: bool,
}

impl ProcessCorrector {
    pub fn new(cmd: &str) -> Self {
        Self {
            cmd: cmd.trim().to_string(),
            inner: Mutex::new(None),
        }
    }

    fn split_cmd(cmd: &str) -> (String, Vec<String>) {
        // Simple argv split on spaces (no shell) — keeps injection surface empty.
        let mut parts = cmd.split_whitespace();
        let prog = parts.next().unwrap_or("").to_string();
        let args = parts.map(|s| s.to_string()).collect();
        (prog, args)
    }

    async fn spawn(cmd: &str) -> Result<ProcHandle, SpellError> {
        let (prog, args) = Self::split_cmd(cmd);
        if prog.is_empty() {
            return Err(SpellError::Failed("llm: empty command".into()));
        }
        let mut child = Command::new(&prog)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| SpellError::Failed(format!("llm spawn '{cmd}': {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SpellError::Failed("llm: no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SpellError::Failed("llm: no stdout".into()))?;
        Ok(ProcHandle {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            used: false,
        })
    }

    /// One request/response over an existing handle. Leaves the process alive.
    async fn exchange(handle: &mut ProcHandle, line: &str) -> Result<String, SpellError> {
        let timeout = if handle.used {
            REQUEST_TIMEOUT
        } else {
            COLD_TIMEOUT
        };
        Self::exchange_with_timeout(handle, line, timeout).await
    }

    /// Same as [`Self::exchange`] but with an explicit deadline
    /// (whole-text proofreading needs more than the single-word budget).
    async fn exchange_with_timeout(
        handle: &mut ProcHandle,
        line: &str,
        timeout: Duration,
    ) -> Result<String, SpellError> {
        handle
            .stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| SpellError::Failed(format!("llm stdin: {e}")))?;
        handle
            .stdin
            .write_all(b"\n")
            .await
            .map_err(|e| SpellError::Failed(format!("llm stdin: {e}")))?;
        handle
            .stdin
            .flush()
            .await
            .map_err(|e| SpellError::Failed(format!("llm flush: {e}")))?;

        let deadline = tokio::time::Instant::now() + timeout;

        // Skip non-answer lines (ready handshake, logs). Need one real reply.
        loop {
            let mut out = String::new();
            let left = deadline
                .saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return Err(SpellError::Failed(format!(
                    "llm timeout after {timeout:?}"
                )));
            }
            let n = tokio::time::timeout(left, handle.stdout.read_line(&mut out))
                .await
                .map_err(|_| SpellError::Failed(format!("llm timeout after {timeout:?}")))?
                .map_err(|e| SpellError::Failed(format!("llm read: {e}")))?;
            if n == 0 {
                return Err(SpellError::Failed("llm: unexpected EOF".into()));
            }
            let text = out.trim_end_matches('\n').trim().to_string();
            if text.is_empty() {
                continue;
            }
            // Greeting / status — not a spell answer.
            if text.contains("\"ready\"") && !text.contains("\"suggestions\"") {
                handle.used = true;
                continue;
            }
            handle.used = true;
            return Ok(text);
        }
    }
}

#[async_trait]
impl ContextCorrector for ProcessCorrector {
    fn name(&self) -> &'static str {
        "llm"
    }

    async fn correct(
        &self,
        word: &str,
        context: &str,
        lang: &str,
    ) -> Result<Vec<ContextFix>, SpellError> {
        if self.cmd.is_empty() {
            return Ok(Vec::new());
        }
        let input = serde_json::json!({
            "word": word,
            "context": context,
            "lang": lang,
        })
        .to_string();

        let mut guard = self.inner.lock().await;
        // Spawn / respawn as needed.
        let need_spawn = match guard.as_mut() {
            None => true,
            Some(h) => matches!(h.child.try_wait(), Ok(Some(_)) | Err(_)),
        };
        if need_spawn {
            *guard = Some(Self::spawn(&self.cmd).await?);
        }

        let handle = guard.as_mut().expect("just spawned");
        let result = Self::exchange(handle, &input).await;
        match result {
            Ok(text) => {
                if text.is_empty() {
                    return Ok(Vec::new());
                }
                Ok(parse_process_output(&text, word))
            }
            Err(e) => {
                // Drop the child so the next word gets a fresh process.
                warn!("llm corrector: {e} — restarting");
                *guard = None;
                Ok(Vec::new())
            }
        }
    }

    /// Whole-text proofread for the «LLM-fix selection» hotkey.
    async fn correct_text(&self, text: &str, lang: &str) -> Result<String, SpellError> {
        if self.cmd.is_empty() {
            return Ok(text.to_string());
        }
        let input = serde_json::json!({
            "cmd": "fix_text",
            "text": text,
            "lang": lang,
        })
        .to_string();

        let mut guard = self.inner.lock().await;
        let need_spawn = match guard.as_mut() {
            None => true,
            Some(h) => matches!(h.child.try_wait(), Ok(Some(_)) | Err(_)),
        };
        if need_spawn {
            *guard = Some(Self::spawn(&self.cmd).await?);
        }
        let handle = guard.as_mut().expect("just spawned");
        // Whole-text proofreading takes much longer than a single-word fix.
        match Self::exchange_with_timeout(handle, &input, FIX_TEXT_TIMEOUT).await {
            Ok(reply) => {
                debug!("fix_text reply ({} B): {:.200}", reply.len(), reply);
                let fixed = parse_fix_text_reply(&reply, text);
                Ok(fixed)
            }
            Err(e) => {
                warn!("llm correct_text: {e} — restarting");
                *guard = None;
                Err(SpellError::Failed(format!("llm: {e}")))
            }
        }
    }
}

/// Whole-text LLM calls generate many tokens — give the model room.
const FIX_TEXT_TIMEOUT: Duration = Duration::from_secs(8);

/// Parse a whole-text fix reply: `{"text":"..."}` or the raw corrected text.
/// Tiny models often append commentary after the fixed text — keep only the
/// first line / sentence.
fn parse_fix_text_reply(reply: &str, original: &str) -> String {
    let reply = reply.trim();
    if reply.is_empty() {
        return original.to_string();
    }
    let body = if let Ok(v) = serde_json::from_str::<serde_json::Value>(reply) {
        if v.get("is_ok").and_then(|x| x.as_bool()).unwrap_or(false)
            && v.get("text").is_none()
        {
            return original.to_string();
        }
        v.get("text")
            .and_then(|x| x.as_str())
            .unwrap_or(original)
            .to_string()
    } else {
        reply.to_string()
    };

    // Keep the first non-empty line — that is the corrected text. The rest is
    // usually model commentary ("The text has been corrected…") or a degenerate
    // few-shot echo loop.
    let first = body
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with("Text:") && !l.starts_with("Fixed:"))
        .unwrap_or(original);
    let first = first
        .strip_prefix("Fixed:")
        .or_else(|| first.strip_prefix("fixed:"))
        .or_else(|| first.strip_prefix("Исправлено:"))
        .unwrap_or(first)
        .trim();
    // Drop a trailing commentary fragment after a sentence end + capital word.
    let first = split_before_commentary(first);
    if first.is_empty() || first == original {
        // First line was garbage equal to input — try any later "clean" line.
        for l in body.lines().map(|l| l.trim()).skip(1) {
            if l.is_empty() || l.starts_with("Text:") || l.starts_with("Fixed:") {
                continue;
            }
            let l = split_before_commentary(l);
            if !l.is_empty() && l != original && l.chars().all(|c| !c.is_control()) {
                return l.to_string();
            }
        }
        return original.to_string();
    }
    first.to_string()
}

/// Cut off trailing LLM commentary ("The text has been corrected…").
fn split_before_commentary(s: &str) -> String {
    for marker in [
        "\nThe text",
        "\nThe corrected",
        "\nИсправлено",
        "\nCommentary",
        " The text has been",
        " The corrected text",
    ] {
        if let Some(i) = s.find(marker) {
            return s[..i].trim().to_string();
        }
    }
    s.trim().to_string()
}

fn parse_process_output(text: &str, word: &str) -> Vec<ContextFix> {
    // JSON protocol first.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        if v.get("is_ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            return vec![ContextFix {
                text: word.to_string(),
                confidence: 0.9,
                is_ok: true,
            }];
        }
        let mut out = Vec::new();
        if let Some(arr) = v.get("suggestions").and_then(|s| s.as_array()) {
            for s in arr.iter().take(5) {
                if let Some(t) = s.as_str() {
                    let t = extract_target_word(t, word);
                    if !t.is_empty() && t != word {
                        out.push(ContextFix {
                            text: t,
                            confidence: 0.9,
                            is_ok: false,
                        });
                    }
                }
            }
        }
        return out;
    }
    // Plain lines — still only one word per fix (never a full-sentence rewrite).
    text.lines()
        .map(|l| extract_target_word(l, word))
        .filter(|l| !l.is_empty() && l != word)
        .take(5)
        .map(|t| ContextFix {
            text: t,
            confidence: 0.75,
            is_ok: false,
        })
        .collect()
}

/// Pull a single replacement word out of a free-form reply.
///
/// LLMs often echo the whole sentence; we only replace ONE word (the
/// target), so keep the token closest to the original — never rewrite
/// the sentence («слова рвутся» otherwise).
fn extract_target_word(reply: &str, target: &str) -> String {
    let reply = reply.trim();
    if reply.is_empty() {
        return String::new();
    }
    // Single token: pass through as-is (punctuation stripping is the
    // caller's job — some backends use markers like `helo!`).
    if !reply.chars().any(char::is_whitespace) {
        return reply.to_string();
    }
    let clean = |s: &str| {
        s.trim()
            .trim_matches(|c: char| {
                matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\'' | '«' | '»' | '(' | ')' | '[' | ']' | '{' | '}' | '\\')
            })
            .to_string()
    };
    let mut best: Option<(usize, String)> = None;
    for tok in reply.split_whitespace() {
        let w = clean(tok);
        if w.is_empty() || w.eq_ignore_ascii_case(target) {
            continue;
        }
        if !w.chars().all(|c| c.is_alphabetic()) {
            continue;
        }
        let d = switchfix_domain::edit_distance(target, &w);
        if d == 0 {
            continue;
        }
        if best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
            best = Some((d, w));
        }
    }
    if let Some((_, w)) = best {
        return w;
    }
    clean(reply.split_whitespace().last().unwrap_or(reply))
}

// ---------------------------------------------------------------------------
// Minimal HTTP form-POST (localhost LanguageTool). No TLS, no extra crates.
// ---------------------------------------------------------------------------

mod reqwest_like {
    use super::*;
    use tokio::net::TcpStream;

    pub struct Client;

    impl Client {
        pub fn new() -> Self {
            Self
        }

        pub async fn post_form(
            &self,
            url: &str,
            body: &str,
            timeout: Duration,
        ) -> Result<String, String> {
            let rest = url
                .strip_prefix("http://")
                .ok_or_else(|| format!("only http:// supported: {url}"))?;
            let (hostport, path) = match rest.find('/') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, "/"),
            };
            let (host, port) = match hostport.rsplit_once(':') {
                Some((h, p)) => (h, p.parse::<u16>().map_err(|e| format!("port: {e}"))?),
                None => (hostport, 80u16),
            };
            if host != "127.0.0.1" && host != "localhost" && host != "::1" {
                return Err(format!("refusing non-local LanguageTool host '{host}'"));
            }

            let fut = async {
                let mut stream = TcpStream::connect((host, port))
                    .await
                    .map_err(|e| format!("connect {host}:{port}: {e}"))?;
                let req = format!(
                    "POST {path} HTTP/1.1\r\nHost: {hostport}\r\n\
                     Content-Type: application/x-www-form-urlencoded\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(req.as_bytes())
                    .await
                    .map_err(|e| format!("write: {e}"))?;
                let mut resp = Vec::new();
                stream
                    .read_to_end(&mut resp)
                    .await
                    .map_err(|e| format!("read: {e}"))?;
                let text = String::from_utf8_lossy(&resp).to_string();
                // Split headers / body.
                let body = text
                    .split_once("\r\n\r\n")
                    .map(|(_, b)| b.to_string())
                    .unwrap_or(text);
                // Chunked encoding: keep payload after size line if present.
                Ok::<String, String>(dechunk(&body))
            };
            tokio::time::timeout(timeout, fut)
                .await
                .map_err(|_| "timeout".to_string())?
        }
    }

    fn dechunk(body: &str) -> String {
        // LanguageTool often uses chunked transfer; naive dechunk.
        if !body.contains("\r\n") {
            return body.to_string();
        }
        let mut out = String::new();
        let mut rest = body;
        while let Some((size_line, tail)) = rest.split_once("\r\n") {
            let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let (chunk, next) = if tail.len() >= size {
                (&tail[..size], &tail[size..])
            } else {
                (tail, "")
            };
            out.push_str(chunk);
            rest = next.trim_start_matches("\r\n");
        }
        if out.is_empty() {
            body.to_string()
        } else {
            out
        }
    }
}

fn form_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the context corrector from config fields. `None` when both empty / backend=dict.
pub fn build_context_corrector(
    backend: &str,
    lt_url: &str,
    llm_cmd: &str,
) -> Option<Box<dyn ContextCorrector>> {
    if backend == "dict" {
        return None;
    }
    let lt = lt_url.trim();
    let llm = llm_cmd.trim();
    if !llm.is_empty() {
        debug!(cmd = %llm, "spell context: process corrector");
        return Some(Box::new(ProcessCorrector::new(llm)));
    }
    if !lt.is_empty() {
        debug!(url = %lt, "spell context: languagetool");
        return Some(Box::new(LanguageToolCorrector::new(lt)));
    }
    if backend == "hybrid" {
        // Hybrid without external engine still gets better dict neighbors
        // (handled in SpellService) — no context corrector required.
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_process_json_suggestions() {
        let fixes = parse_process_output(r#"{"suggestions":["привет","приветик"],"is_ok":false}"#, "приветт");
        assert_eq!(fixes.len(), 2);
        assert_eq!(fixes[0].text, "привет");
        assert!(!fixes[0].is_ok);
    }

    #[test]
    fn parse_process_json_is_ok() {
        let fixes = parse_process_output(r#"{"is_ok":true}"#, "привет");
        assert_eq!(fixes.len(), 1);
        assert!(fixes[0].is_ok);
    }

    #[test]
    fn parse_process_plain_lines() {
        let fixes = parse_process_output("hello\nhallo\n", "helo");
        assert_eq!(fixes.len(), 2);
        assert_eq!(fixes[0].text, "hello");
    }

    #[test]
    fn extract_target_word_from_sentence_reply() {
        // LLM echoed a full sentence — keep only the fix, never the rewrite.
        assert_eq!(extract_target_word("это было новое слово", "новае"), "новое");
        assert_eq!(extract_target_word("он начала говорить", "ачала"), "начала");
        // Single word is passed through.
        assert_eq!(extract_target_word("привет", "приветт"), "привет");
        // Last-word fallback when nothing is close enough.
        assert_eq!(extract_target_word("да", "xyzzy"), "да");
    }

    #[test]
    fn parse_process_sentence_reply_becomes_single_word() {
        let fixes = parse_process_output(
            r#"{"suggestions":["это было новое слово"],"is_ok":false}"#,
            "новае",
        );
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].text, "новое", "sentence reply must not tear the text");
    }

    #[test]
    fn lt_parse_veto_when_no_match() {
        let fixes = parse_lt_response(r#"{"matches":[]}"#, "привет", "привет").unwrap();
        assert!(fixes[0].is_ok);
    }

    #[test]
    fn lt_parse_word_replacements() {
        let json = r#"{"matches":[{"offset":0,"length":6,"replacements":[{"value":"привет"}]}]}"#;
        let fixes = parse_lt_response(json, "приветт", "приветт").unwrap();
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].text, "привет");
        assert!(!fixes[0].is_ok);
    }

    #[test]
    fn build_prefers_llm_over_lt() {
        let c = build_context_corrector("hybrid", "http://127.0.0.1:8010", "my-llm");
        assert!(c.is_some());
        assert_eq!(c.unwrap().name(), "llm");
    }

    #[test]
    fn build_dict_disables_context() {
        assert!(build_context_corrector("dict", "http://127.0.0.1:8010", "x").is_none());
    }

    #[test]
    fn form_encode_basic() {
        assert_eq!(form_urlencode("a b"), "a+b");
        assert_eq!(form_urlencode("ё"), "%D1%91");
    }

    #[tokio::test]
    async fn process_corrector_skips_ready_and_stays_alive() {
        // Tiny echo-server style corrector: ready line + answers per request.
        let script = std::env::temp_dir().join(format!(
            "llm-spell-fake-{}.py",
            std::process::id()
        ));
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import sys, json
print(json.dumps({"ready": True, "backend": "fake"}), flush=True)
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    w = req.get("word", "")
    if w == "okword":
        print(json.dumps({"is_ok": True}), flush=True)
    else:
        print(json.dumps({"suggestions": [w + "!"], "is_ok": False}), flush=True)
"#,
        )
        .unwrap();
        let c = ProcessCorrector::new(&format!("python3 {}", script.display()));
        let f1 = c.correct("helo", "", "en").await.unwrap();
        assert_eq!(f1.len(), 1);
        assert_eq!(f1[0].text, "helo!");
        let f2 = c.correct("okword", "", "en").await.unwrap();
        assert!(f2[0].is_ok);
        // Same process: a third call still works (no respawn crash).
        let f3 = c.correct("again", "", "en").await.unwrap();
        assert_eq!(f3[0].text, "again!");
        let _ = std::fs::remove_file(&script);
    }
}
