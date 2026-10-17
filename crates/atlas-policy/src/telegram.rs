use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_TELEGRAM_BOT_TOKEN: &str = "8527817116:AAGLcyMG8cgCQhJOUeXknTZDsAblyTqY8NU";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Approved {
        approver: String,
        timestamp: DateTime<Utc>,
        notes: Option<String>,
    },
    Rejected {
        reason: String,
        timestamp: DateTime<Utc>,
    },
    ReplaceRequested {
        story_id: String,
        reason: String,
    },
    ResearchRequested {
        topic: String,
        notes: String,
    },
    Pending,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposedNewsStory {
    pub story_id: String,
    pub category: String, // Anime, Cartoons, Adult Animation, Industry, Upcoming
    pub headline: String,
    pub factual_summary: String,
    pub source_url: String,
    pub announcement_date: String,
    pub estimated_duration_mins: f64,
    pub editorial_angle: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EpisodeSlateProposal {
    pub episode_id: String,
    pub episode_number: u32,
    pub target_duration_mins: f64,
    pub stories: Vec<ProposedNewsStory>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CopyrightReviewCard {
    pub review_id: String,
    pub episode_id: String,
    pub asset_id: String,
    pub asset_description: String,
    pub rights_holder: String,
    pub source_url: String,
    pub intended_duration_secs: f64,
    pub editorial_purpose: String,
    pub fair_use_summary: String,
    pub risk_level: String, // Low, Medium, High
    pub proposed_replacement: String,
}

pub struct TelegramApprovalGate {
    pub bot_token: String,
    pub chat_id: Option<i64>,
    client: reqwest::Client,
}

impl TelegramApprovalGate {
    pub fn new(token: Option<String>, chat_id: Option<i64>) -> Self {
        let bot_token = token
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
            .unwrap_or_else(|| DEFAULT_TELEGRAM_BOT_TOKEN.to_string());

        Self {
            bot_token,
            chat_id,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn format_slate_markdown(&self, slate: &EpisodeSlateProposal) -> String {
        let mut msg = "📺 *CLEARLY DRAWN NEWS — EDITORIAL SLATE APPROVAL*\n".to_string();
        msg.push_str(&format!(
            "Episode: *#{}* (`{}`) | Target: *{:.1} mins* (15–30m Range)\n",
            slate.episode_number, slate.episode_id, slate.target_duration_mins
        ));
        msg.push_str("═══════════════════════════════════════\n\n");

        for (i, s) in slate.stories.iter().enumerate() {
            let verified_badge = if s.verified { "✅ Verified" } else { "⚠️ Needs Source" };
            msg.push_str(&format!(
                "*{}. [{}] {}* (_{:.1}m_ | {})\n",
                i + 1, s.category, s.headline, s.estimated_duration_mins, verified_badge
            ));
            msg.push_str(&format!("• *Facts:* {}\n", s.factual_summary));
            msg.push_str(&format!("• *Angle:* {}\n", s.editorial_angle));
            msg.push_str(&format!("• *Source:* [{}]({})\n\n", s.announcement_date, s.source_url));
        }

        msg.push_str("⚠️ *MANDATORY APPROVAL GATE*: Production is paused until you click an action below.");
        msg
    }

    pub async fn get_bot_info(&self) -> Result<serde_json::Value, String> {
        let url = format!("https://api.telegram.org/bot{}/getMe", self.bot_token);
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(json.get("result").cloned().unwrap_or_default())
        } else {
            Err(format!("Telegram API error: {}", json))
        }
    }

    pub async fn send_slate(&mut self, slate: &EpisodeSlateProposal) -> Result<i64, String> {
        let chat_id = self.resolve_chat_id().await?;
        let text = self.format_slate_markdown(slate);
        let ep = &slate.episode_id;

        let keyboard = serde_json::json!({
            "inline_keyboard": [
                [
                    {"text": "✅ Approve Full Slate", "callback_data": format!("approve:{}", ep)},
                    {"text": "❌ Reject Slate", "callback_data": format!("reject:{}", ep)}
                ],
                [
                    {"text": "🔄 Replace Story", "callback_data": format!("replace:{}", ep)},
                    {"text": "🔍 More Research", "callback_data": format!("research:{}", ep)}
                ]
            ]
        });

        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown",
            "disable_web_page_preview": true,
            "reply_markup": keyboard
        });

        let resp = self.client.post(&url).json(&payload).send().await.map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            let msg_id = json["result"]["message_id"].as_i64().unwrap_or(0);
            Ok(msg_id)
        } else {
            Err(format!("Failed sending Telegram slate: {}", json))
        }
    }

    pub async fn send_copyright_card(&mut self, card: &CopyrightReviewCard) -> Result<i64, String> {
        let chat_id = self.resolve_chat_id().await?;
        let text = format!(
            "🛡️ *CLEARLY DRAWN NEWS — MEDIA RIGHTS REVIEW*\n\
             Asset: `{}` | Episode: `{}`\n\
             ═══════════════════════════════════════\n\
             • *Description:* {}\n\
             • *Rights Holder:* {}\n\
             • *Duration:* {:.1}s\n\
             • *Editorial Purpose:* {}\n\
             • *Fair Use Factor:* {}\n\
             • *Risk Assessment:* *{}*\n\
             • *Safe Replacement:* {}\n\n\
             *Action Required:* Clearance approval required before final master rendering.",
            card.asset_id, card.episode_id, card.asset_description, card.rights_holder,
            card.intended_duration_secs, card.editorial_purpose, card.fair_use_summary,
            card.risk_level, card.proposed_replacement
        );

        let id = &card.review_id;
        let keyboard = serde_json::json!({
            "inline_keyboard": [
                [
                    {"text": "✅ Approve Documented Use", "callback_data": format!("c_approve:{}", id)},
                    {"text": "🔄 Replace Asset", "callback_data": format!("c_replace:{}", id)}
                ],
                [
                    {"text": "📄 Request License", "callback_data": format!("c_license:{}", id)},
                    {"text": "❌ Reject Asset", "callback_data": format!("c_reject:{}", id)}
                ]
            ]
        });

        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "Markdown",
            "disable_web_page_preview": true,
            "reply_markup": keyboard
        });

        let resp = self.client.post(&url).json(&payload).send().await.map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(json["result"]["message_id"].as_i64().unwrap_or(0))
        } else {
            Err(format!("Failed sending copyright review card: {}", json))
        }
    }

    pub async fn poll_approval(
        &self,
        episode_id: &str,
        max_poll_iterations: u32,
    ) -> Result<ApprovalDecision, String> {
        let url = format!("https://api.telegram.org/bot{}/getUpdates", self.bot_token);
        let mut last_update_id: i64 = 0;

        for _ in 0..max_poll_iterations {
            let query = if last_update_id > 0 {
                format!("?offset={}&timeout=2", last_update_id + 1)
            } else {
                "?timeout=2".to_string()
            };

            let resp = self.client.get(format!("{url}{query}")).send().await.map_err(|e| e.to_string())?;
            let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

            if let Some(updates) = json.get("result").and_then(|r| r.as_array()) {
                for u in updates {
                    if let Some(uid) = u.get("update_id").and_then(|v| v.as_i64()) {
                        last_update_id = last_update_id.max(uid);
                    }

                    if let Some(cb) = u.get("callback_query") {
                        let data = cb.get("data").and_then(|v| v.as_str()).unwrap_or_default();
                        let from_user = cb["from"]["username"]
                            .as_str()
                            .or_else(|| cb["from"]["first_name"].as_str())
                            .unwrap_or("Telegram User")
                            .to_string();

                        // Acknowledge callback query
                        if let Some(cb_id) = cb.get("id").and_then(|v| v.as_str()) {
                            let ans_url = format!("https://api.telegram.org/bot{}/answerCallbackQuery", self.bot_token);
                            let _ = self.client.post(&ans_url).json(&serde_json::json!({
                                "callback_query_id": cb_id,
                                "text": "Decision recorded in Clearly Drawn News production ledger."
                            })).send().await;
                        }

                        if data.starts_with("approve:") && data.ends_with(episode_id) {
                            return Ok(ApprovalDecision::Approved {
                                approver: from_user,
                                timestamp: Utc::now(),
                                notes: None,
                            });
                        } else if data.starts_with("reject:") && data.ends_with(episode_id) {
                            return Ok(ApprovalDecision::Rejected {
                                reason: format!("Rejected by {from_user} via Telegram"),
                                timestamp: Utc::now(),
                            });
                        } else if data.starts_with("replace:") && data.ends_with(episode_id) {
                            return Ok(ApprovalDecision::ReplaceRequested {
                                story_id: "selected".into(),
                                reason: format!("Requested replacement by {from_user}"),
                            });
                        } else if data.starts_with("research:") && data.ends_with(episode_id) {
                            return Ok(ApprovalDecision::ResearchRequested {
                                topic: episode_id.to_string(),
                                notes: format!("More research requested by {from_user}"),
                            });
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }

        Ok(ApprovalDecision::Pending)
    }

    async fn resolve_chat_id(&mut self) -> Result<i64, String> {
        if let Some(id) = self.chat_id {
            return Ok(id);
        }
        if let Ok(env_id) = std::env::var("TELEGRAM_CHAT_ID") {
            if let Ok(parsed) = env_id.parse::<i64>() {
                self.chat_id = Some(parsed);
                return Ok(parsed);
            }
        }

        // Try extracting chat_id from recent incoming messages
        let url = format!("https://api.telegram.org/bot{}/getUpdates?limit=10", self.bot_token);
        let resp = self.client.get(&url).send().await.map_err(|e| e.to_string())?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

        if let Some(updates) = json.get("result").and_then(|r| r.as_array()) {
            for u in updates.iter().rev() {
                if let Some(chat_id) = u["message"]["chat"]["id"].as_i64() {
                    self.chat_id = Some(chat_id);
                    return Ok(chat_id);
                }
                if let Some(chat_id) = u["callback_query"]["message"]["chat"]["id"].as_i64() {
                    self.chat_id = Some(chat_id);
                    return Ok(chat_id);
                }
            }
        }

        Err("No active Telegram chat ID found. Send any message or /start to @youtubepipline2146_bot to link your chat, or set TELEGRAM_CHAT_ID.".to_string())
    }
}
