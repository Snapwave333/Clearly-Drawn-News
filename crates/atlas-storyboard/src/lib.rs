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

#[derive(Clone)]
pub struct DirectorClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl DirectorClient {
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()))
    }

    /// Builds the client from `ATLAS_DIRECTOR_URL`, `ATLAS_DIRECTOR_MODEL`, and the first key found in
    /// `ATLAS_DIRECTOR_API_KEY`, then `ANTHROPIC_API_KEY` (Anthropic URL only), then `NVIDIA_API_KEY`.
    /// `ATLAS_DIRECTOR_TIMEOUT_SECS` overrides the request timeout (180 s for Anthropic, 60 s otherwise).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let endpoint = lookup("ATLAS_DIRECTOR_URL").unwrap_or_else(|| DEFAULT_DIRECTOR_URL.into());
        let anthropic = is_anthropic_endpoint(&endpoint);
        let api_key = lookup("ATLAS_DIRECTOR_API_KEY")
            .or_else(|| if anthropic { lookup("ANTHROPIC_API_KEY") } else { None })
            .or_else(|| lookup("NVIDIA_API_KEY"))?;
        let model = lookup("ATLAS_DIRECTOR_MODEL").unwrap_or_else(|| {
            if anthropic { DEFAULT_ANTHROPIC_MODEL.into() } else { DEFAULT_NVIDIA_MODEL.into() }
        });
        let timeout = lookup("ATLAS_DIRECTOR_TIMEOUT_SECS")
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .unwrap_or(if anthropic { 180 } else { 60 });
        Some(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .build()
                .unwrap_or_default(),
            endpoint,
            api_key,
            model,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn is_anthropic(&self) -> bool {
        is_anthropic_endpoint(&self.endpoint)
    }

    /// OpenAI-style chat body. Anthropic's compatible endpoint rejects `temperature` on current Claude
    /// models and ignores `response_format`, so both are omitted there and `max_tokens` is set instead.
    fn chat_body(&self, system: &str, user: &str, temperature: f64) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });
        if self.is_anthropic() {
            body["max_tokens"] = serde_json::json!(ANTHROPIC_MAX_TOKENS);
        } else {
            body["temperature"] = serde_json::json!(temperature);
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }
        body
    }

    async fn chat(&self, system: &str, user: &str, temperature: f64) -> AtlasResult<String> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&self.chat_body(system, user, temperature))
            .send()
            .await
            .map_err(director_error)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.chars().take(300).collect();
            return Err(AtlasError::Config(format!("Creative director returned {status}: {detail}")));
        }
        let payload: serde_json::Value = response.json().await.map_err(director_error)?;
        payload["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AtlasError::Config("Director response omitted JSON content".into()))
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
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "temperature": 0.55,
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": "You adapt approved long-form educational storyboards into faithful vertical shorts. Return valid JSON only."},
                    {"role": "user", "content": format!("Create exactly {beat_count} beats for a sub-60-second vertical short derived only from this approved long-form storyboard: {long_json}. Preserve its thesis and facts. Choose the strongest hook, essential mechanism, misconception or risk, and synthesis. Rewrite tightly instead of adding claims. Every narration sentence must be 8 to 24 words. Keep Luma's character direction and use portrait-friendly focal terms. Return the same StoryboardPlan schema.")}
                ]
            }))
            .send()
            .await
            .map_err(director_error)?
            .error_for_status()
            .map_err(director_error)?;
        let payload: serde_json::Value = response.json().await.map_err(director_error)?;
        let content = payload["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                AtlasError::Config("Short adapter response omitted JSON content".into())
            })?;
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
const DEFAULT_NVIDIA_MODEL: &str = "meta/llama-3.2-11b-vision-instruct";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";
const ANTHROPIC_MAX_TOKENS: u32 = 8192;

fn director_error(error: reqwest::Error) -> AtlasError {
    AtlasError::Config(format!("Creative director request failed: {}", error.without_url()))
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
    fn nvidia_director_keeps_temperature_and_ignores_anthropic_key() {
        assert!(DirectorClient::from_lookup(lookup(&[("ANTHROPIC_API_KEY", "k")])).is_none());
        let director = DirectorClient::from_lookup(lookup(&[("NVIDIA_API_KEY", "k")])).unwrap();
        assert!(!director.is_anthropic());
        let body = director.chat_body("sys", "user", 0.6);
        assert_eq!(body["temperature"], 0.6);
        assert_eq!(body["response_format"]["type"], "json_object");
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
