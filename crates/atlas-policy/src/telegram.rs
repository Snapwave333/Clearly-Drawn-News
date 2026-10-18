use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};


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
    #[serde(default)]
    pub secondary_source_url: Option<String>,
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
    bot_token: String,
    pub chat_id: Option<i64>,
    client: reqwest::Client,
}

impl TelegramApprovalGate {
    pub fn new(token: Option<String>, chat_id: Option<i64>) -> Result<Self, String> {
        let bot_token = token
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                "Telegram bot token missing: set TELEGRAM_BOT_TOKEN or pass --token".to_string()
            })?;

        Ok(Self {
            bot_token,
            chat_id,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        })
    }

    pub fn format_slate_message(&self, slate: &EpisodeSlateProposal) -> String {
        for (fact_chars, include_angle) in [(320, true), (200, true), (160, false), (90, false)] {
            let message = render_slate(slate, slate.stories.len(), fact_chars, include_angle);
            if message.chars().count() <= TELEGRAM_MESSAGE_BUDGET {
                return message;
            }
        }
        let mut shown = slate.stories.len();
        loop {
            let message = render_slate(slate, shown, 90, false);
            if message.chars().count() <= TELEGRAM_MESSAGE_BUDGET || shown == 0 {
                return message;
            }
            shown -= 1;
        }
    }

    pub async fn get_bot_info(&self) -> Result<serde_json::Value, String> {
        let url = format!("https://api.telegram.org/bot{}/getMe", self.bot_token);
        let resp = self.client.get(&url).send().await.map_err(redact_error)?;
        let json: serde_json::Value = resp.json().await.map_err(redact_error)?;
        if json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(json.get("result").cloned().unwrap_or_default())
        } else {
            Err(format!("Telegram API error: {}", json))
        }
    }

    pub async fn send_slate(&mut self, slate: &EpisodeSlateProposal) -> Result<i64, String> {
        let chat_id = self.resolve_chat_id().await?;
        let text = self.format_slate_message(slate);
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
            "parse_mode": "HTML",
            "disable_web_page_preview": true,
            "reply_markup": keyboard
        });

        let resp = self.client.post(&url).json(&payload).send().await.map_err(redact_error)?;
        let json: serde_json::Value = resp.json().await.map_err(redact_error)?;
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
            "🛡️ <b>CLEARLY DRAWN NEWS — MEDIA RIGHTS REVIEW</b>\n\
             Asset: <code>{}</code> | Episode: <code>{}</code>\n\
             ═══════════════════════════════════════\n\
             • <b>Description:</b> {}\n\
             • <b>Rights Holder:</b> {}\n\
             • <b>Duration:</b> {:.1}s\n\
             • <b>Editorial Purpose:</b> {}\n\
             • <b>Fair Use Factor:</b> {}\n\
             • <b>Risk Assessment:</b> <b>{}</b>\n\
             • <b>Safe Replacement:</b> {}\n\n\
             <b>Action Required:</b> Clearance approval required before final master rendering.",
            escape_html(&card.asset_id), escape_html(&card.episode_id), escape_html(&card.asset_description),
            escape_html(&card.rights_holder), card.intended_duration_secs, escape_html(&card.editorial_purpose),
            escape_html(&card.fair_use_summary), escape_html(&card.risk_level), escape_html(&card.proposed_replacement)
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
            "parse_mode": "HTML",
            "disable_web_page_preview": true,
            "reply_markup": keyboard
        });

        let resp = self.client.post(&url).json(&payload).send().await.map_err(redact_error)?;
        let json: serde_json::Value = resp.json().await.map_err(redact_error)?;
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

            let resp = self.client.get(format!("{url}{query}")).send().await.map_err(redact_error)?;
            let json: serde_json::Value = resp.json().await.map_err(redact_error)?;

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

    /// Sends a plain informational message (no buttons, nothing to approve).
    pub async fn send_notice(&mut self, text: &str) -> Result<i64, String> {
        let chat_id = self.resolve_chat_id().await?;
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": clip(text, TELEGRAM_MESSAGE_BUDGET),
        });
        let resp = self.client.post(&url).json(&payload).send().await.map_err(redact_error)?;
        let json: serde_json::Value = resp.json().await.map_err(redact_error)?;
        if json.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(json["result"]["message_id"].as_i64().unwrap_or(0))
        } else {
            Err("Telegram rejected the notice".to_string())
        }
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
        let resp = self.client.get(&url).send().await.map_err(redact_error)?;
        let json: serde_json::Value = resp.json().await.map_err(redact_error)?;

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

const TELEGRAM_MESSAGE_BUDGET: usize = 3900;
const NOTICE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Best-effort Telegram notice for unattended runs. Never fails: a missing or placeholder
/// `TELEGRAM_BOT_TOKEN`, an unlinked chat, a network error, or a slow API (20 s cap) only
/// returns `false`. Uses `TELEGRAM_CHAT_ID` when set.
pub async fn notify_best_effort(text: &str) -> bool {
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").ok().and_then(|id| id.trim().parse().ok());
    let Ok(mut gate) = TelegramApprovalGate::new(None, chat_id) else {
        return false;
    };
    matches!(tokio::time::timeout(NOTICE_TIMEOUT, gate.send_notice(text)).await, Ok(Ok(_)))
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut = text.chars().take(max_chars).collect::<String>();
    format!("{}…", cut.trim_end())
}

fn render_slate(slate: &EpisodeSlateProposal, shown: usize, fact_chars: usize, include_angle: bool) -> String {
    let corroborated = slate.stories.iter().filter(|s| s.secondary_source_url.is_some()).count();
    let mut msg = format!(
        "📺 <b>CLEARLY DRAWN NEWS — EDITORIAL SLATE APPROVAL</b>\n\
         Episode: <b>#{}</b> (<code>{}</code>) | Target: <b>{:.1} mins</b> (15–30m range)\n\
         Sourcing: <b>{}/{}</b> stories confirmed by two outlets\n\
         ═══════════════════════════════════════\n\n",
        slate.episode_number,
        escape_html(&slate.episode_id),
        slate.target_duration_mins,
        corroborated,
        slate.stories.len()
    );
    for (i, s) in slate.stories.iter().take(shown).enumerate() {
        let badge = match (&s.secondary_source_url, s.verified) {
            (_, false) => "⚠️ Needs source",
            (Some(_), true) => "✅✅ Two sources",
            (None, true) => "✅ Single source",
        };
        msg.push_str(&format!(
            "<b>{}. [{}] {}</b> (<i>{:.1}m</i> | {})\n",
            i + 1,
            escape_html(&s.category),
            escape_html(&s.headline),
            s.estimated_duration_mins,
            badge
        ));
        msg.push_str(&format!("• <b>Facts:</b> {}\n", escape_html(&clip(&s.factual_summary, fact_chars))));
        if include_angle {
            msg.push_str(&format!("• <b>Angle:</b> {}\n", escape_html(&s.editorial_angle)));
        }
        msg.push_str(&format!(
            "• <b>Sources:</b> <a href=\"{}\">{}</a>",
            escape_html(&s.source_url),
            escape_html(&s.announcement_date)
        ));
        if let Some(second) = &s.secondary_source_url {
            msg.push_str(&format!(" · <a href=\"{}\">second outlet</a>", escape_html(second)));
        }
        msg.push_str("\n\n");
    }
    if shown < slate.stories.len() {
        msg.push_str(&format!(
            "… plus {} more stories in the episode's <code>slate.json</code>.\n\n",
            slate.stories.len() - shown
        ));
    }
    msg.push_str("⚠️ <b>MANDATORY APPROVAL GATE</b>: Production is paused until you click an action below.");
    msg
}

fn redact_error(error: reqwest::Error) -> String {
    error.without_url().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story(headline: &str, summary: &str, second: Option<&str>) -> ProposedNewsStory {
        ProposedNewsStory {
            story_id: headline.into(),
            category: "Anime".into(),
            headline: headline.into(),
            factual_summary: summary.into(),
            source_url: "https://www.animenewsnetwork.com/news/a?b=1&c=2".into(),
            secondary_source_url: second.map(Into::into),
            announcement_date: "2026-10-17".into(),
            estimated_duration_mins: 3.5,
            editorial_angle: "Marcus hypes; Elena checks the schedule.".into(),
            verified: true,
        }
    }

    #[test]
    fn slate_escapes_live_headlines_and_shows_sourcing() {
        let gate = TelegramApprovalGate::new(Some("1:x".into()), None).unwrap();
        let slate = EpisodeSlateProposal {
            episode_id: "EP-002".into(),
            episode_number: 2,
            target_duration_mins: 22.0,
            stories: vec![
                story("<Oshi_no_Ko> *Season 3* & [Movie]", "Doga Kobo confirmed it.", Some("https://www.cartoonbrew.com/x")),
                story("Toei Update", "Single report.", None),
            ],
        };
        let msg = gate.format_slate_message(&slate);
        assert!(msg.contains("&lt;Oshi_no_Ko&gt; *Season 3* &amp; [Movie]"));
        assert!(msg.contains("1/2</b> stories confirmed by two outlets"));
        assert!(msg.contains("✅✅ Two sources"));
        assert!(msg.contains("✅ Single source"));
        assert!(msg.contains("href=\"https://www.animenewsnetwork.com/news/a?b=1&amp;c=2\""));
        assert!(!msg.contains("<Oshi"));
    }

    #[test]
    fn slate_fits_telegram_limit() {
        let gate = TelegramApprovalGate::new(Some("1:x".into()), None).unwrap();
        let long = "Detailed confirmed facts about the production. ".repeat(40);
        let slate = EpisodeSlateProposal {
            episode_id: "EP-003".into(),
            episode_number: 3,
            target_duration_mins: 25.0,
            stories: (0..30).map(|i| story(&format!("Story {i}"), &long, Some("https://x.test"))).collect(),
        };
        let msg = gate.format_slate_message(&slate);
        assert!(msg.chars().count() <= TELEGRAM_MESSAGE_BUDGET);
        assert!(msg.contains("more stories in the episode"));
        assert!(msg.ends_with("click an action below."));
    }

    #[test]
    fn gate_requires_a_bot_token() {
        let missing = TelegramApprovalGate::new(Some("  ".into()), None);
        if std::env::var("TELEGRAM_BOT_TOKEN").is_err() {
            assert!(missing.is_err());
        }
        assert!(TelegramApprovalGate::new(Some("123:abc".into()), Some(42)).is_ok());
    }
}
