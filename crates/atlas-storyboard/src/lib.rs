use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};

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
    pub fn validate(&self, expected_beats: usize) -> AtlasResult<()> {
        if self.beats.len() != expected_beats {
            return Err(AtlasError::Config(format!(
                "Storyboard returned {} beats; expected {expected_beats}",
                self.beats.len()
            )));
        }
        for (index, beat) in self.beats.iter().enumerate() {
            let words = beat.narration.split_whitespace().count();
            if !(8..=42).contains(&words) {
                return Err(AtlasError::Config(format!(
                    "Beat {} narration has {words} words; expected 8 to 42",
                    index + 1
                )));
            }
            if beat.title.trim().is_empty() || beat.title.chars().count() > 64 {
                return Err(AtlasError::Config(format!(
                    "Beat {} title is empty or too long",
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

pub struct DirectorClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl DirectorClient {
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("ATLAS_DIRECTOR_URL")
            .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1/chat/completions".into());
        let api_key = std::env::var("ATLAS_DIRECTOR_API_KEY")
            .or_else(|_| std::env::var("NVIDIA_API_KEY"))
            .ok()?;
        let model = std::env::var("ATLAS_DIRECTOR_MODEL")
            .unwrap_or_else(|_| "nvidia/nemotron-3-ultra-550b-a55b".into());
        Some(Self {
            client: reqwest::Client::new(),
            endpoint,
            api_key,
            model,
        })
    }

    pub async fn create_plan(
        &self,
        topic: &str,
        format: &str,
        beat_count: usize,
    ) -> AtlasResult<StoryboardPlan> {
        let prompt = director_prompt(topic, format, beat_count);
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "temperature": 0.72,
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": "You are the creative director for an educational animation studio. Return valid JSON only."},
                    {"role": "user", "content": prompt}
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
            .ok_or_else(|| AtlasError::Config("Director response omitted JSON content".into()))?;
        let plan: StoryboardPlan = serde_json::from_str(strip_json_fence(content))
            .map_err(|error| AtlasError::Config(format!("Invalid director storyboard: {error}")))?;
        plan.validate(beat_count)?;
        Ok(plan)
    }

    pub async fn create_topic(&self, existing_titles: &[String]) -> AtlasResult<TopicIdea> {
        let existing = existing_titles.join(" | ");
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "temperature": 0.85,
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": "You select accurate evergreen topics for an educational animation channel. Return valid JSON only."},
                    {"role": "user", "content": format!("Choose one fresh educational topic with a visually explainable mechanism, broad curiosity appeal, and reliable public facts. Avoid news, politics, medical advice, financial advice, dangerous instructions, and every existing title. Existing titles: {existing}. Return exactly title and rationale.")}
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
                AtlasError::Config("Topic director response omitted JSON content".into())
            })?;
        let idea: TopicIdea = serde_json::from_str(strip_json_fence(content))
            .map_err(|error| AtlasError::Config(format!("Invalid topic idea: {error}")))?;
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
        let plan: StoryboardPlan = serde_json::from_str(strip_json_fence(content))
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
        "Create a {beat_count}-beat {format} educational video about {topic}. Build a causal teaching arc with a strong hook, concrete mental model, misconception or limit, and memorable synthesis. Every beat needs one narration sentence, one to four focal_terms, a specific stock-media broll_query, and character direction for Luma, an expressive light-bulb narrator. scene_pattern must be one of hook, parts, flow, split_path, comparison, scale, timeline, cause_effect, risk, cycle. emotion must be curious, excited, explaining, concerned, or confident. gesture must be point_left, point_right, present, think, warn, or celebrate. Morphs are optional and must clarify the concept. A morph silhouette is exactly eleven ordered perimeter points, each [x,y] normalized from -1 to 1, forming a recognizable simple icon while Luma keeps its face, gloves, filament, and screw base. Return an object with topic, thesis, and beats. Avoid generic filler, repeated scenes, unsupported claims, and visual metaphors unrelated to the narration."
    )
}

fn strip_json_fence(value: &str) -> &str {
    value
        .trim()
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or_else(|| value.trim())
}

fn director_error(error: reqwest::Error) -> AtlasError {
    AtlasError::Config(format!("Creative director request failed: {error}"))
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

    #[test]
    fn accepts_data_defined_morph() {
        assert!(plan(vec![[0.0, 0.0]; 11]).validate(1).is_ok());
    }

    #[test]
    fn rejects_invalid_morph_geometry() {
        assert!(plan(vec![[0.0, 0.0]; 10]).validate(1).is_err());
    }
}
