use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};

pub mod pronunciation;
pub mod story_script;
pub mod two_anchor_script;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryboardPlan {
    pub topic: String,
    pub thesis: String,
    pub beats: Vec<StoryboardBeat>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopicIdea {
    pub title: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryboardBeat {
    #[serde(default)]
    pub title: String,
    pub narration: String,
    pub scene_pattern: ScenePattern,
    pub focal_terms: Vec<String>,
    pub broll_query: String,
    pub character: CharacterDirection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePattern {
    Hook,
    Parts,
    Flow,
    SplitPath,
    Comparison,
    Scale,
    Timeline,
    CauseEffect,
    Risk,
    Cycle,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterDirection {
    pub emotion: Emotion,
    pub gesture: Gesture,
    pub acting_goal: String,
    pub morph: Option<MorphDirection>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Emotion {
    Curious,
    Excited,
    Explaining,
    Concerned,
    Confident,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    PointLeft,
    PointRight,
    Present,
    Think,
    Warn,
    Celebrate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MorphDirection {
    pub label: String,
    pub silhouette: Vec<[f32; 2]>,
}

impl StoryboardPlan {
    pub fn validate(&mut self, expected_beats: usize) -> AtlasResult<()> {
        if self.beats.len() != expected_beats {
            return Err(AtlasError::Config(format!(
                "Storyboard returned {} beats; expected {expected_beats}",
                self.beats.len()
            )));
        }
        for (index, beat) in self.beats.iter_mut().enumerate() {
            if beat.title.trim().is_empty() {
                beat.title = beat
                    .focal_terms
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("BEAT {}", index + 1))
                    .to_uppercase();
            }
            let words = beat.narration.split_whitespace().count();
            if !(8..=120).contains(&words) {
                return Err(AtlasError::Config(format!(
                    "Beat {} narration has {words} words; expected 8 to 120",
                    index + 1
                )));
            }
            if beat.title.chars().count() > 64 {
                return Err(AtlasError::Config(format!(
                    "Beat {} title is too long",
                    index + 1
                )));
            }
            if beat.focal_terms.is_empty() || beat.focal_terms.len() > 4 {
                return Err(AtlasError::Config(format!(
                    "Beat {} must contain one to four focal terms",
                    index + 1
                )));
            }
            if let Some(morph) = &beat.character.morph
                && (morph.silhouette.len() != 11
                    || morph
                        .silhouette
                        .iter()
                        .flatten()
                        .any(|value| !(-1.0..=1.0).contains(value)))
            {
                return Err(AtlasError::Config(format!(
                    "Beat {} morph must contain eleven normalized points",
                    index + 1
                )));
            }
        }
        Ok(())
    }
}

/// One chat-completions backend in the director's failover chain.
#[derive(Clone, Debug)]
struct Provider {
    /// "anthropic", "nvidia-nim" or "custom".
    name: &'static str,
    endpoint: String,
    api_key: String,
    model: String,
}

impl Provider {
    fn is_anthropic(&self) -> bool {
        is_anthropic_endpoint(&self.endpoint)
    }

    fn is_nim(&self) -> bool {
        self.endpoint.contains("integrate.api.nvidia.com")
    }

    /// OpenAI-style chat body with each backend's quirks:
    /// * Anthropic's compatible endpoint rejects `temperature` on current Claude models and ignores
    ///   `response_format`; both are omitted and `max_tokens` is set.
    /// * NVIDIA NIM: `max_tokens` is set (the default is short enough to cut a script off), the
    ///   temperature is clamped to the model's accepted range (Kimi K2 models are pinned at their
    ///   recommended 0.6), and `response_format` is sent only to models that support it.
    fn body(&self, system: &str, user: &str, temperature: f64) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });
        if self.is_anthropic() {
            body["max_tokens"] = serde_json::json!(ANTHROPIC_MAX_TOKENS);
        } else if self.is_nim() {
            body["max_tokens"] = serde_json::json!(NIM_MAX_TOKENS);
            body["temperature"] = serde_json::json!(nim_temperature(&self.model, temperature));
            if nim_supports_json_mode(&self.model) {
                body["response_format"] = serde_json::json!({"type": "json_object"});
            }
        } else {
            body["temperature"] = serde_json::json!(temperature);
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }
        body
    }
}

/// Temperature a NIM model accepts: Kimi K2 models are tuned for (and on some hosts fixed at) 0.6;
/// everything else is clamped to 0..=1, since several NIM models reject values above 1.
fn nim_temperature(model: &str, requested: f64) -> f64 {
    if model.contains("kimi-k2") { 0.6 } else { requested.clamp(0.0, 1.0) }
}

/// JSON mode is unreliable on NIM's reasoning models, which then answer with an empty content.
/// The reply is fence-stripped either way, so it is only requested where it is known to work.
fn nim_supports_json_mode(model: &str) -> bool {
    !(model.contains("reasoning") || model.contains("deepseek") || model.contains("kimi"))
}

/// Why a provider call failed, and whether the chain should move on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    /// Credits used up, billing or quota errors, or a rejected key: skip this provider for the
    /// rest of the process.
    Exhausted,
    /// Rate limits, overload, server errors, timeouts: try the next provider for this call.
    Transient,
    /// Anything else (a malformed request): also try the next provider, whose quirks differ.
    Other,
}

fn classify(status: u16, body: &str) -> Failure {
    let lower = body.to_lowercase();
    let billing = ["credit balance", "credits", "billing", "quota", "insufficient", "payment", "exceeded your", "usage limit", "spend limit"]
        .iter()
        .any(|w| lower.contains(w));
    match status {
        401 | 402 => Failure::Exhausted,
        400 | 403 | 429 if billing => Failure::Exhausted,
        403 => Failure::Exhausted,
        408 | 409 | 425 | 429 | 500..=599 => Failure::Transient,
        _ => Failure::Other,
    }
}

/// The creative director: a failover chain of chat providers. Anthropic (Eli's Claude credits) is
/// tried first, then NVIDIA NIM's free models. A provider that reports billing, quota or key
/// errors is skipped for the rest of the run; every call logs which provider handled it.
#[derive(Clone)]
pub struct DirectorClient {
    client: reqwest::Client,
    providers: Vec<Provider>,
    exhausted: std::sync::Arc<Vec<std::sync::atomic::AtomicBool>>,
    log_path: Option<std::path::PathBuf>,
}

impl DirectorClient {
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()))
    }

    /// Builds the chain.
    ///
    /// * Primary: `ATLAS_DIRECTOR_URL` + `ATLAS_DIRECTOR_MODEL` + the first key found in
    ///   `ATLAS_DIRECTOR_API_KEY`, then `ANTHROPIC_API_KEY` (Anthropic URL only), then
    ///   `NVIDIA_API_KEY`. With no URL set, Anthropic is primary when `ANTHROPIC_API_KEY` exists,
    ///   otherwise NVIDIA NIM.
    /// * Fallbacks: NVIDIA NIM (`integrate.api.nvidia.com/v1`) with `NVIDIA_API_KEY`, models from
    ///   `ATLAS_DIRECTOR_FALLBACK_MODELS` (comma-separated; default Kimi K2.6 then DeepSeek V4.1
    ///   Flash). `ATLAS_DIRECTOR_FALLBACK=off` disables them.
    /// * `ATLAS_DIRECTOR_TIMEOUT_SECS` overrides the request timeout (180 s; 60 s for a NIM-only chain).
    /// * `ATLAS_DIRECTOR_LOG` names a JSONL file that gets one line per call (provider, model,
    ///   outcome, milliseconds); every call is also logged to stderr.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let nvidia_key = lookup("NVIDIA_API_KEY");
        let explicit_url = lookup("ATLAS_DIRECTOR_URL");
        let endpoint = explicit_url.clone().unwrap_or_else(|| {
            if lookup("ANTHROPIC_API_KEY").is_some() {
                ANTHROPIC_URL.into()
            } else {
                DEFAULT_DIRECTOR_URL.into()
            }
        });
        let anthropic = is_anthropic_endpoint(&endpoint);
        let mut providers = Vec::new();
        let primary_key = lookup("ATLAS_DIRECTOR_API_KEY")
            .or_else(|| if anthropic { lookup("ANTHROPIC_API_KEY") } else { None })
            .or_else(|| if anthropic { None } else { nvidia_key.clone() });
        let fallback_models: Vec<String> = lookup("ATLAS_DIRECTOR_FALLBACK_MODELS")
            .map(|v| v.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect())
            .unwrap_or_else(|| DEFAULT_NIM_FALLBACK_MODELS.iter().map(|m| m.to_string()).collect());
        if let Some(api_key) = primary_key {
            let model = lookup("ATLAS_DIRECTOR_MODEL").unwrap_or_else(|| {
                if anthropic { DEFAULT_ANTHROPIC_MODEL.into() } else { fallback_models.first().cloned().unwrap_or_else(|| DEFAULT_NVIDIA_MODEL.into()) }
            });
            let name = if anthropic { "anthropic" } else if endpoint.contains("integrate.api.nvidia.com") { "nvidia-nim" } else { "custom" };
            providers.push(Provider { name, endpoint: endpoint.clone(), api_key, model });
        }
        let fallback_on = !lookup("ATLAS_DIRECTOR_FALLBACK").is_some_and(|v| v.trim().eq_ignore_ascii_case("off"));
        if fallback_on && let Some(key) = nvidia_key {
            for model in fallback_models {
                if !providers.iter().any(|p| p.endpoint == DEFAULT_DIRECTOR_URL && p.model == model) {
                    providers.push(Provider { name: "nvidia-nim", endpoint: DEFAULT_DIRECTOR_URL.into(), api_key: key.clone(), model });
                }
            }
        }
        if providers.is_empty() {
            return None;
        }
        let all_nim = providers.iter().all(Provider::is_nim);
        let timeout = lookup("ATLAS_DIRECTOR_TIMEOUT_SECS")
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .unwrap_or(if all_nim { 60 } else { 180 });
        let exhausted = std::sync::Arc::new(providers.iter().map(|_| std::sync::atomic::AtomicBool::new(false)).collect());
        Some(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .build()
                .unwrap_or_default(),
            providers,
            exhausted,
            log_path: lookup("ATLAS_DIRECTOR_LOG").map(std::path::PathBuf::from),
        })
    }

    /// Writes the per-call log to this JSONL file as well (overrides `ATLAS_DIRECTOR_LOG`).
    pub fn with_log(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.log_path = Some(path.into());
        self
    }

    fn active(&self) -> impl Iterator<Item = (usize, &Provider)> {
        self.providers
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.exhausted[*i].load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Model of the provider that will be tried first.
    pub fn model(&self) -> &str {
        self.active().next().map(|(_, p)| p.model.as_str()).unwrap_or_else(|| self.providers[0].model.as_str())
    }

    pub fn is_anthropic(&self) -> bool {
        self.active().next().is_some_and(|(_, p)| p.is_anthropic())
    }

    /// "anthropic:claude-sonnet-5-5 -> nvidia-nim:moonshotai/kimi-k2.6 -> ..." (no keys).
    pub fn chain(&self) -> String {
        self.providers.iter().map(|p| format!("{}:{}", p.name, p.model)).collect::<Vec<_>>().join(" -> ")
    }

    #[cfg(test)]
    fn chat_body(&self, system: &str, user: &str, temperature: f64) -> serde_json::Value {
        self.providers[0].body(system, user, temperature)
    }

    fn log_call(&self, provider: &Provider, purpose: &str, outcome: &str, millis: u128) {
        eprintln!("  [director] {purpose}: {}:{} {outcome} ({millis} ms)", provider.name, provider.model);
        if let Some(path) = &self.log_path {
            let line = serde_json::json!({
                "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                "purpose": purpose,
                "provider": provider.name,
                "model": provider.model,
                "outcome": outcome,
                "ms": millis,
            });
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    async fn call_one(&self, provider: &Provider, system: &str, user: &str, temperature: f64) -> Result<String, (Failure, String)> {
        let response = self
            .client
            .post(&provider.endpoint)
            .bearer_auth(&provider.api_key)
            .json(&provider.body(system, user, temperature))
            .send()
            .await
            .map_err(|e| (Failure::Transient, format!("request failed: {}", e.without_url())))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.chars().take(300).collect();
            return Err((classify(status.as_u16(), &body), format!("returned {status}: {detail}")));
        }
        let payload: serde_json::Value = response.json().await.map_err(|e| (Failure::Transient, format!("unreadable reply: {}", e.without_url())))?;
        let content = payload["choices"][0]["message"]["content"].as_str().map(strip_think).unwrap_or_default();
        if content.trim().is_empty() {
            return Err((Failure::Other, "reply had no content".into()));
        }
        Ok(content)
    }

    /// Sends one chat call down the chain until a provider answers.
    async fn chat(&self, system: &str, user: &str, temperature: f64) -> AtlasResult<String> {
        let purpose: String = system.split(['.', ':']).next().unwrap_or(system).chars().take(60).collect();
        let mut errors = Vec::new();
        for (index, provider) in self.providers.iter().enumerate() {
            if self.exhausted[index].load(std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            let started = std::time::Instant::now();
            match self.call_one(provider, system, user, temperature).await {
                Ok(content) => {
                    let outcome = if errors.is_empty() { "ok".to_string() } else { format!("ok after failover ({})", errors.len()) };
                    self.log_call(provider, &purpose, &outcome, started.elapsed().as_millis());
                    return Ok(content);
                }
                Err((failure, detail)) => {
                    if failure == Failure::Exhausted {
                        self.exhausted[index].store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    let outcome = match failure {
                        Failure::Exhausted => format!("unavailable for this run, failing over: {detail}"),
                        Failure::Transient => format!("failed, trying next: {detail}"),
                        Failure::Other => format!("error, trying next: {detail}"),
                    };
                    self.log_call(provider, &purpose, &outcome, started.elapsed().as_millis());
                    errors.push(format!("{}:{} {detail}", provider.name, provider.model));
                }
            }
        }
        Err(AtlasError::Config(format!("Creative director: every provider failed: {}", errors.join(" | "))))
    }

    pub async fn create_plan(
        &self,
        topic: &str,
        format: &str,
        beat_count: usize,
    ) -> AtlasResult<StoryboardPlan> {
        let prompt = director_prompt(topic, format, beat_count);
        let content = self
            .chat(
                "You are the creative director for an educational animation studio. Return valid JSON only.",
                &prompt,
                0.72,
            )
            .await?;
        let content = content.as_str();
        let mut plan: StoryboardPlan = serde_json::from_str(strip_json_fence(content))
            .map_err(|error| AtlasError::Config(format!("Invalid director storyboard: {error}")))?;
        plan.validate(beat_count)?;
        Ok(plan)
    }

    pub async fn complete_json(&self, system: &str, prompt: &str, temperature: f64) -> AtlasResult<serde_json::Value> {
        let content = self.chat(system, prompt, temperature).await?;
        let content = content.as_str();
        serde_json::from_str(strip_json_fence(content))
            .map_err(|error| AtlasError::Config(format!("Director returned invalid JSON: {error}")))
    }

    pub async fn create_topic(&self, existing_titles: &[String]) -> AtlasResult<TopicIdea> {
        let existing = existing_titles.join(" | ");
        let content = self
            .chat(
                "You select accurate evergreen topics for an educational animation channel. Return valid JSON only.",
                &format!("Choose one fresh educational topic with a visually explainable mechanism, broad curiosity appeal, and reliable public facts. Avoid news, politics, medical advice, financial advice, dangerous instructions, and every existing title. Existing titles: {existing}. Return exactly title and rationale."),
                0.85,
            )
            .await?;
        let content = content.as_str();
        let stripped = strip_json_fence(content);
        let idea: TopicIdea = if let Ok(parsed) = serde_json::from_str::<TopicIdea>(stripped) {
            parsed
        } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(stripped) {
            let title = val["title"].as_str().or_else(|| val["text"].as_str()).unwrap_or("How do semiconductors power modern electronics?");
            let rationale = val["rationale"].as_str().unwrap_or("Bandgap physics and solid-state silicon switches.");
            TopicIdea {
                title: title.to_string(),
                rationale: rationale.to_string(),
            }
        } else {
            let first_line = content.lines().next().unwrap_or("How do solar panels generate electricity?");
            TopicIdea {
                title: first_line.trim_matches(|c| c == '"' || c == '{' || c == '}').trim().to_string(),
                rationale: "Physics of photon absorption and charge carriers.".into(),
            }
        };
        if idea.title.trim().is_empty()
            || idea.title.chars().count() > 100
            || existing_titles
                .iter()
                .any(|title| title.eq_ignore_ascii_case(idea.title.trim()))
        {
            return Err(AtlasError::Config(
                "Topic director returned an empty, duplicate, or oversized title".into(),
            ));
        }
        Ok(idea)
    }

    pub async fn create_short_plan(
        &self,
        long_plan: &StoryboardPlan,
        beat_count: usize,
    ) -> AtlasResult<StoryboardPlan> {
        let long_json = serde_json::to_string(long_plan)?;
        let content = self
            .chat(
                "You adapt approved long-form educational storyboards into faithful vertical shorts. Return valid JSON only.",
                &format!("Create exactly {beat_count} beats for a sub-60-second vertical short derived only from this approved long-form storyboard: {long_json}. Preserve its thesis and facts. Choose the strongest hook, essential mechanism, misconception or risk, and synthesis. Rewrite tightly instead of adding claims. Every narration sentence must be 8 to 24 words. Keep Luma's character direction and use portrait-friendly focal terms. Return the same StoryboardPlan schema."),
                0.55,
            )
            .await?;
        let content = content.as_str();
        let mut plan: StoryboardPlan = serde_json::from_str(strip_json_fence(content))
            .map_err(|error| AtlasError::Config(format!("Invalid derived short plan: {error}")))?;
        plan.validate(beat_count)?;
        if plan
            .beats
            .iter()
            .any(|beat| beat.narration.split_whitespace().count() > 24)
        {
            return Err(AtlasError::Config(
                "Derived short contains narration longer than 24 words".into(),
            ));
        }
        Ok(plan)
    }
}

fn director_prompt(topic: &str, format: &str, beat_count: usize) -> String {
    format!(
        "Create a {beat_count}-beat {format} educational video about {topic}. Build a causal teaching arc with a strong hook, concrete mental model, misconception or limit, and memorable synthesis. Every beat needs a title (a short punchy headline in uppercase, 2-5 words), an educational narration explanation (for long-form videos provide a thorough, informative paragraph of 40-90 words; for shorts provide 12-24 words), one to four focal_terms, a specific stock-media broll_query, and character direction for Luma, an expressive light-bulb narrator. scene_pattern must be one of hook, parts, flow, split_path, comparison, scale, timeline, cause_effect, risk, cycle. emotion must be curious, excited, explaining, concerned, or confident. gesture must be point_left, point_right, present, think, warn, or celebrate. Morphs are optional and must clarify the concept. A morph silhouette is exactly eleven ordered perimeter points, each [x,y] normalized from -1 to 1, forming a recognizable simple icon while Luma keeps its face, gloves, filament, and screw base. Return an object with topic, thesis, and beats. Avoid generic filler, repeated scenes, unsupported claims, and visual metaphors unrelated to the narration."
    )
}

/// Extracts the JSON object from a model reply: strips a ```json fence and any prose around the
/// outermost `{ ... }` (Claude does not honor `response_format`).
fn strip_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.trim_end().strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    match (unfenced.find('{'), unfenced.rfind('}')) {
        (Some(open), Some(close)) if close > open => &unfenced[open..=close],
        _ => unfenced,
    }
}

fn is_anthropic_endpoint(endpoint: &str) -> bool {
    endpoint.contains("api.anthropic.com")
}

const DEFAULT_DIRECTOR_URL: &str = "https://integrate.api.nvidia.com/v1/chat/completions";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/chat/completions";
/// NIM fallbacks for scripts and translation, strongest first: Kimi K2.6 (strong long-form
/// writing and Japanese/Korean/Chinese), then DeepSeek V4.1 Flash.
const DEFAULT_NIM_FALLBACK_MODELS: &[&str] = &["moonshotai/kimi-k2.6", "deepseek-ai/deepseek-v4.1-flash"];
const DEFAULT_NVIDIA_MODEL: &str = "moonshotai/kimi-k2.6";
const NIM_MAX_TOKENS: u32 = 8192;
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";
const ANTHROPIC_MAX_TOKENS: u32 = 8192;

/// Drops a reasoning model's `<think>...</think>` preamble.
fn strip_think(content: &str) -> String {
    match (content.find("<think>"), content.find("</think>")) {
        (Some(open), Some(close)) if close > open => format!("{}{}", &content[..open], &content[close + "</think>".len()..]).trim().to_string(),
        _ => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(points: Vec<[f32; 2]>) -> StoryboardPlan {
        StoryboardPlan {
            topic: "test".into(),
            thesis: "test thesis".into(),
            beats: vec![StoryboardBeat {
                title: "A CLEAR BEAT".into(),
                narration:
                    "This sentence contains enough useful words to pass the validation gate today."
                        .into(),
                scene_pattern: ScenePattern::Flow,
                focal_terms: vec!["input".into(), "output".into()],
                broll_query: "specific visual".into(),
                character: CharacterDirection {
                    emotion: Emotion::Explaining,
                    gesture: Gesture::Present,
                    acting_goal: "Trace the transfer".into(),
                    morph: Some(MorphDirection {
                        label: "arrow".into(),
                        silhouette: points,
                    }),
                },
            }],
        }
    }

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string())
    }

    #[test]
    fn anthropic_director_omits_temperature_and_sets_max_tokens() {
        let director = DirectorClient::from_lookup(lookup(&[
            ("ATLAS_DIRECTOR_URL", "https://api.anthropic.com/v1/chat/completions"),
            ("ANTHROPIC_API_KEY", "k"),
        ]))
        .unwrap();
        assert!(director.is_anthropic());
        assert_eq!(director.model(), DEFAULT_ANTHROPIC_MODEL);
        let body = director.chat_body("sys", "user", 0.6);
        assert!(body.get("temperature").is_none());
        assert!(body.get("response_format").is_none());
        assert_eq!(body["max_tokens"], ANTHROPIC_MAX_TOKENS);
        assert_eq!(body["messages"][0]["role"], "system");
    }

    #[test]
    fn nim_only_chain_handles_temperature_quirks() {
        let director = DirectorClient::from_lookup(lookup(&[("NVIDIA_API_KEY", "k")])).unwrap();
        assert!(!director.is_anthropic());
        assert_eq!(director.chain(), "nvidia-nim:moonshotai/kimi-k2.6 -> nvidia-nim:deepseek-ai/deepseek-v4.1-flash");
        let body = director.chat_body("sys", "user", 0.9);
        assert_eq!(body["temperature"], 0.6, "Kimi K2 is pinned at 0.6");
        assert!(body.get("response_format").is_none());
        assert_eq!(body["max_tokens"], NIM_MAX_TOKENS);
        assert_eq!(nim_temperature("deepseek-ai/deepseek-v4.1-flash", 1.4), 1.0);
        assert_eq!(nim_temperature("z-ai/glm-5.3", 0.2), 0.2);
    }

    #[test]
    fn anthropic_first_then_nim_failover_chain() {
        let director = DirectorClient::from_lookup(lookup(&[("ANTHROPIC_API_KEY", "a"), ("NVIDIA_API_KEY", "n")])).unwrap();
        assert_eq!(
            director.chain(),
            "anthropic:claude-sonnet-5-5 -> nvidia-nim:moonshotai/kimi-k2.6 -> nvidia-nim:deepseek-ai/deepseek-v4.1-flash"
        );
        assert!(director.is_anthropic());
        // Anthropic alone (no NVIDIA key yet) still works, with no fallback.
        let alone = DirectorClient::from_lookup(lookup(&[("ANTHROPIC_API_KEY", "a")])).unwrap();
        assert_eq!(alone.chain(), "anthropic:claude-sonnet-5-5");
        let off = DirectorClient::from_lookup(lookup(&[("ANTHROPIC_API_KEY", "a"), ("NVIDIA_API_KEY", "n"), ("ATLAS_DIRECTOR_FALLBACK", "off")])).unwrap();
        assert_eq!(off.chain(), "anthropic:claude-sonnet-5-5");
        let custom = DirectorClient::from_lookup(lookup(&[("NVIDIA_API_KEY", "n"), ("ATLAS_DIRECTOR_FALLBACK_MODELS", "z-ai/glm-5.3")])).unwrap();
        assert_eq!(custom.chain(), "nvidia-nim:z-ai/glm-5.3");
        assert!(DirectorClient::from_lookup(lookup(&[])).is_none());
    }

    #[test]
    fn billing_and_quota_errors_exhaust_a_provider() {
        assert_eq!(classify(400, r#"{"error":{"message":"Your credit balance is too low to access the Anthropic API."}}"#), Failure::Exhausted);
        assert_eq!(classify(402, ""), Failure::Exhausted);
        assert_eq!(classify(429, "You have exceeded your monthly quota"), Failure::Exhausted);
        assert_eq!(classify(401, "invalid x-api-key"), Failure::Exhausted);
        assert_eq!(classify(429, "rate limited, slow down"), Failure::Transient);
        assert_eq!(classify(529, "overloaded"), Failure::Transient);
        assert_eq!(classify(400, "unknown field"), Failure::Other);
        assert_eq!(strip_think("<think>hmm</think>\n{\"a\":1}"), "{\"a\":1}");
    }

    #[test]
    fn json_is_extracted_from_fences_and_prose() {
        assert_eq!(strip_json_fence("```json\n{\"a\": 1}\n```"), "{\"a\": 1}");
        assert_eq!(strip_json_fence("```\n{\"a\": 1}\n```"), "{\"a\": 1}");
        assert_eq!(strip_json_fence("Here you go:\n{\"a\": {\"b\": 2}}\nThanks"), "{\"a\": {\"b\": 2}}");
        assert_eq!(strip_json_fence("no json"), "no json");
    }

    #[test]
    fn accepts_data_defined_morph() {
        assert!(plan(vec![[0.0, 0.0]; 11]).validate(1).is_ok());
    }

    #[test]
    fn rejects_invalid_morph_geometry() {
        assert!(plan(vec![[0.0, 0.0]; 10]).validate(1).is_err());
    }
}
