//! External OpenAI-compatible chat-completions corrector.
//!
//! Sends the selected text to `POST {base_url}/v1/chat/completions` with an
//! optional Bearer token and a configurable system prompt. Used only by the
//! «LLM-fix selection» hotkey — never for auto-replace.

use std::time::Duration;

use async_trait::async_trait;
use mistype_app::config::Llm;
use mistype_app::ports::{LlmProbe, TextCorrector, TextFixError};
use tracing::debug;

pub struct OpenAiCorrector {
    base: String,
    token: String,
    system_prompt: String,
    model: String,
    timeout: Duration,
}

impl OpenAiCorrector {
    pub fn new(cfg: &Llm) -> Self {
        Self {
            base: cfg.base_url.trim_end_matches('/').to_string(),
            token: cfg.api_token.clone(),
            system_prompt: cfg.system_prompt.clone(),
            model: cfg.model.clone(),
            timeout: Duration::from_millis(cfg.timeout_ms),
        }
    }

    pub fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base)
    }

    pub fn models_endpoint(base: &str) -> String {
        format!("{}/v1/models", base.trim_end_matches('/'))
    }

    /// Pure request builder — unit-testable without HTTP.
    pub fn build_request_json(text: &str, system: &str, model: &str) -> serde_json::Value {
        let mut body = serde_json::json!({
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": text },
            ],
        });
        if !model.is_empty() {
            body["model"] = serde_json::Value::String(model.to_string());
        }
        body
    }

    /// Pure response parser — `choices[0].message.content`.
    pub fn parse_response(body: &str) -> Result<String, TextFixError> {
        let v: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| TextFixError::Parse(format!("json: {e}")))?;
        let content = v
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|ch| ch.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| TextFixError::Parse("missing choices[0].message.content".into()))?;
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err(TextFixError::Api("empty completion".into()));
        }
        Ok(trimmed.to_string())
    }

    /// Pure parser for `GET /v1/models` → `data[].id`.
    pub fn parse_models(body: &str) -> Result<Vec<String>, TextFixError> {
        let v: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| TextFixError::Parse(format!("json: {e}")))?;
        let arr = v
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| TextFixError::Parse("missing data[]".into()))?;
        let mut out: Vec<String> = arr
            .iter()
            .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    fn get_json(url: &str, token: &str, timeout: Duration) -> Result<String, TextFixError> {
        debug!(url = %url, "openai corrector: GET");
        let mut req = ureq::get(url).timeout(timeout);
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        let resp = req.call().map_err(|e| TextFixError::Http(format!("{e}")))?;
        let status = resp.status();
        if !(200..300).contains(&status) {
            let snippet: String = resp
                .into_string()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            return Err(TextFixError::Api(format!("HTTP {status}: {snippet}")));
        }
        resp.into_string()
            .map_err(|e| TextFixError::Http(format!("read body: {e}")))
    }

    fn post_chat(&self, body: &serde_json::Value) -> Result<String, TextFixError> {
        let url = self.endpoint();
        debug!(url = %url, model = %self.model, "openai corrector: POST");
        let mut req = ureq::post(&url)
            .timeout(self.timeout)
            .set("Content-Type", "application/json");
        if !self.token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.token));
        }
        let resp = req
            .send_string(&body.to_string())
            .map_err(|e| TextFixError::Http(format!("{e}")))?;
        let status = resp.status();
        if !(200..300).contains(&status) {
            let snippet: String = resp
                .into_string()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            return Err(TextFixError::Api(format!("HTTP {status}: {snippet}")));
        }
        let text = resp
            .into_string()
            .map_err(|e| TextFixError::Http(format!("read body: {e}")))?;
        Self::parse_response(&text)
    }
}

#[async_trait]
impl TextCorrector for OpenAiCorrector {
    fn name(&self) -> &'static str {
        "openai"
    }

    async fn correct_text(&self, text: &str) -> Result<String, TextFixError> {
        let body = Self::build_request_json(text, &self.system_prompt, &self.model);
        // Blocking HTTP in a worker thread so the async runtime stays free.
        let base = self.base.clone();
        let token = self.token.clone();
        let system_prompt = self.system_prompt.clone();
        let model = self.model.clone();
        let timeout = self.timeout;
        tokio::task::spawn_blocking(move || {
            let client = OpenAiCorrector {
                base,
                token,
                system_prompt,
                model,
                timeout,
            };
            client.post_chat(&body)
        })
        .await
        .map_err(|e| TextFixError::Http(format!("join: {e}")))?
    }
}

#[async_trait]
impl LlmProbe for OpenAiCorrector {
    async fn list_models(
        &self,
        base_url: &str,
        api_token: &str,
        timeout_ms: u64,
    ) -> Result<Vec<String>, TextFixError> {
        let url = Self::models_endpoint(base_url);
        let token = api_token.to_string();
        let timeout = Duration::from_millis(timeout_ms);
        tokio::task::spawn_blocking(move || {
            let body = Self::get_json(&url, &token, timeout)?;
            Self::parse_models(&body)
        })
        .await
        .map_err(|e| TextFixError::Http(format!("join: {e}")))?
    }

    async fn test_chat(&self, cfg: &Llm) -> Result<String, TextFixError> {
        let probe = OpenAiCorrector::new(cfg);
        // Tiny fixed prompt — just prove the endpoint answers.
        let body = Self::build_request_json("Ответь одним словом: ок", &cfg.system_prompt, &cfg.model);
        tokio::task::spawn_blocking(move || probe.post_chat(&body))
            .await
            .map_err(|e| TextFixError::Http(format!("join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_includes_system_and_user() {
        let body = OpenAiCorrector::build_request_json("приветт", "fix it", "gpt-4o-mini");
        assert_eq!(body["model"], "gpt-4o-mini");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "fix it");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "приветт");
    }

    #[test]
    fn build_request_omits_empty_model() {
        let body = OpenAiCorrector::build_request_json("hi", "sys", "");
        assert!(body.get("model").is_none());
    }

    #[test]
    fn parse_happy_response() {
        let raw = r#"{"choices":[{"message":{"content":"  привет  \n"}}]}"#;
        assert_eq!(OpenAiCorrector::parse_response(raw).unwrap(), "привет");
    }

    #[test]
    fn parse_empty_content_is_error() {
        let raw = r#"{"choices":[{"message":{"content":"   "}}]}"#;
        assert!(matches!(
            OpenAiCorrector::parse_response(raw),
            Err(TextFixError::Api(_))
        ));
    }

    #[test]
    fn parse_missing_choices_is_error() {
        assert!(matches!(
            OpenAiCorrector::parse_response(r#"{"foo":1}"#),
            Err(TextFixError::Parse(_))
        ));
    }

    #[test]
    fn parse_non_json_is_error() {
        assert!(matches!(
            OpenAiCorrector::parse_response("not json"),
            Err(TextFixError::Parse(_))
        ));
    }

    #[test]
    fn endpoint_appends_chat_completions() {
        let cfg = Llm {
            base_url: "https://api.openai.com/".into(),
            ..Default::default()
        };
        let c = OpenAiCorrector::new(&cfg);
        assert_eq!(c.endpoint(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn parse_models_lists_ids_sorted() {
        let raw = r#"{"data":[{"id":"b-model"},{"id":"a-model"},{"id":"a-model"}]}"#;
        assert_eq!(
            OpenAiCorrector::parse_models(raw).unwrap(),
            vec!["a-model".to_string(), "b-model".to_string()]
        );
    }

    #[test]
    fn parse_models_missing_data_is_error() {
        assert!(matches!(
            OpenAiCorrector::parse_models(r#"{"object":"list"}"#),
            Err(TextFixError::Parse(_))
        ));
    }

    #[test]
    fn models_endpoint_joins_base() {
        assert_eq!(
            OpenAiCorrector::models_endpoint("http://127.0.0.1:11434/"),
            "http://127.0.0.1:11434/v1/models"
        );
    }
}
