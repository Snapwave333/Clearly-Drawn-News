use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

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
    pub category: String,
    pub headline: String,
    pub factual_summary: String,
    pub source_url: String,
    pub sources: Vec<String>,
    pub announcement_date: String,
    pub estimated_duration_mins: f64,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EpisodeSlateProposal {
    pub episode_id: String,
    pub episode_number: u32,
    pub target_duration_mins: f64,
    pub stories: Vec<ProposedNewsStory>,
    #[serde(default)]
    pub bulletins: Vec<String>,
}

pub struct TelegramApprovalGate {
    bot_token: String,
    api_base: String,
    pub chat_id: Option<i64>,
    client: reqwest::Client,
    update_offset: i64,
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

impl TelegramApprovalGate {
    pub fn new(token: Option<String>, chat_id: Option<i64>) -> Result<Self, String> {
        let bot_token = token
            .filter(|t| !t.trim().is_empty())
            .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok().filter(|t| !t.trim().is_empty()))
            .ok_or("No Telegram bot token: set TELEGRAM_BOT_TOKEN or pass --token")?;
        let chat_id = chat_id.or_else(|| std::env::var("TELEGRAM_CHAT_ID").ok().and_then(|v| v.trim().parse().ok()));
        let api_base = std::env::var("TELEGRAM_API_BASE")
            .unwrap_or_else(|_| "https://api.telegram.org".into())
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            bot_token,
            api_base,
            chat_id,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            update_offset: 0,
        })
    }

    fn method_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api_base, self.bot_token, method)
    }

    async fn call(&self, method: &str, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        let response = self
            .client
            .post(self.method_url(method))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Telegram {method} failed: {}", e.without_url()))?;
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Telegram {method} returned invalid JSON: {}", e.without_url()))?;
        if json["ok"].as_bool() == Some(true) {
            Ok(json["result"].clone())
        } else {
            Err(format!(
                "Telegram {method} error: {}",
                json["description"].as_str().unwrap_or("unknown error")
            ))
        }
    }

    pub async fn get_bot_info(&self) -> Result<serde_json::Value, String> {
        self.call("getMe", serde_json::json!({})).await
    }

    pub fn format_slate_html(slate: &EpisodeSlateProposal) -> String {
        let mut msg = format!(
            "📺 <b>CLEARLY DRAWN NEWS — SLATE APPROVAL</b>\nEpisode <b>#{}</b> (<code>{}</code>) · target <b>{:.0} min</b>\n\n",
            slate.episode_number,
            escape_html(&slate.episode_id),
            slate.target_duration_mins
        );
        for (i, s) in slate.stories.iter().enumerate() {
            let badge = if s.verified { "✅" } else { "⚠️" };
            msg.push_str(&format!(
                "<b>{}. [{}] {}</b> {badge}\n{}\n<i>{}</i> · <a href=\"{}\">source</a>\n\n",
                i + 1,
                escape_html(&s.category),
                escape_html(&clip(&s.headline, 140)),
                escape_html(&clip(&s.factual_summary, 260)),
                escape_html(&s.sources.join(", ")),
                escape_html(&s.source_url),
            ));
        }
        if !slate.bulletins.is_empty() {
            msg.push_str("<b>Rapid-fire (single source):</b>\n");
            for b in slate.bulletins.iter().take(8) {
                msg.push_str(&format!("• {}\n", escape_html(&clip(b, 120))));
            }
            msg.push('\n');
        }
        msg.push_str("Production is paused until you choose an action below.");
        clip(&msg, 4000)
    }

    pub fn slate_keyboard(slate: &EpisodeSlateProposal) -> serde_json::Value {
        let ep = &slate.episode_id;
        let mut rows = vec![
            vec![
                serde_json::json!({"text": "✅ Approve", "callback_data": format!("approve:{ep}")}),
                serde_json::json!({"text": "❌ Reject", "callback_data": format!("reject:{ep}")}),
            ],
            vec![serde_json::json!({"text": "🔍 More Research", "callback_data": format!("research:{ep}")})],
        ];
        let swaps: Vec<serde_json::Value> = slate
            .stories
            .iter()
            .enumerate()
            .map(|(i, _)| serde_json::json!({"text": format!("🔄 Swap #{}", i + 1), "callback_data": format!("replace:{ep}:{i}")}))
            .collect();
        for chunk in swaps.chunks(3) {
            rows.push(chunk.to_vec());
        }
        serde_json::json!({"inline_keyboard": rows})
    }

    pub async fn send_slate(&mut self, slate: &EpisodeSlateProposal) -> Result<i64, String> {
        let chat_id = self.resolve_chat_id().await?;
        self.skip_pending_updates().await?;
        let result = self
            .call(
                "sendMessage",
                serde_json::json!({
                    "chat_id": chat_id,
                    "text": Self::format_slate_html(slate),
                    "parse_mode": "HTML",
                    "disable_web_page_preview": true,
                    "reply_markup": Self::slate_keyboard(slate)
                }),
            )
            .await?;
        Ok(result["message_id"].as_i64().unwrap_or(0))
    }

    pub async fn send_text(&mut self, text: &str) -> Result<i64, String> {
        let chat_id = self.resolve_chat_id().await?;
        let result = self
            .call(
                "sendMessage",
                serde_json::json!({
                    "chat_id": chat_id,
                    "text": clip(text, 4000),
                    "parse_mode": "HTML",
                    "disable_web_page_preview": true
                }),
            )
            .await?;
        Ok(result["message_id"].as_i64().unwrap_or(0))
    }

    async fn skip_pending_updates(&mut self) -> Result<(), String> {
        let updates = self
            .call("getUpdates", serde_json::json!({"offset": -1, "timeout": 0}))
            .await?;
        if let Some(last) = updates.as_array().and_then(|u| u.last()) {
            self.update_offset = last["update_id"].as_i64().unwrap_or(0) + 1;
        }
        Ok(())
    }

    pub async fn poll_approval(&mut self, slate: &EpisodeSlateProposal, timeout: Duration) -> Result<ApprovalDecision, String> {
        let chat_id = self.resolve_chat_id().await?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs().clamp(1, 25);
            let updates = self
                .call(
                    "getUpdates",
                    serde_json::json!({"offset": self.update_offset, "timeout": remaining, "allowed_updates": ["callback_query"]}),
                )
                .await?;
            for update in updates.as_array().cloned().unwrap_or_default() {
                if let Some(uid) = update["update_id"].as_i64() {
                    self.update_offset = self.update_offset.max(uid + 1);
                }
                let cb = &update["callback_query"];
                if cb.is_null() || cb["message"]["chat"]["id"].as_i64() != Some(chat_id) {
                    continue;
                }
                let data = cb["data"].as_str().unwrap_or_default();
                let Some(decision) = parse_callback(data, slate, cb) else {
                    continue;
                };
                if let Some(cb_id) = cb["id"].as_str() {
                    let _ = self
                        .call(
                            "answerCallbackQuery",
                            serde_json::json!({"callback_query_id": cb_id, "text": "Decision recorded."}),
                        )
                        .await;
                }
                if let Some(message_id) = cb["message"]["message_id"].as_i64() {
                    let _ = self
                        .call(
                            "editMessageReplyMarkup",
                            serde_json::json!({"chat_id": chat_id, "message_id": message_id, "reply_markup": {"inline_keyboard": []}}),
                        )
                        .await;
                }
                return Ok(decision);
            }
        }
        Ok(ApprovalDecision::Pending)
    }

    async fn resolve_chat_id(&mut self) -> Result<i64, String> {
        if let Some(id) = self.chat_id {
            return Ok(id);
        }
        let updates = self.call("getUpdates", serde_json::json!({"limit": 20, "timeout": 0})).await?;
        for u in updates.as_array().cloned().unwrap_or_default().iter().rev() {
            if let Some(id) = u["message"]["chat"]["id"]
                .as_i64()
                .or_else(|| u["callback_query"]["message"]["chat"]["id"].as_i64())
            {
                self.chat_id = Some(id);
                return Ok(id);
            }
        }
        Err("No Telegram chat linked: send /start to the bot from your account, or set TELEGRAM_CHAT_ID.".into())
    }
}

fn parse_callback(data: &str, slate: &EpisodeSlateProposal, cb: &serde_json::Value) -> Option<ApprovalDecision> {
    let from = cb["from"]["username"]
        .as_str()
        .or_else(|| cb["from"]["first_name"].as_str())
        .unwrap_or("Telegram user")
        .to_string();
    let mut parts = data.splitn(3, ':');
    let action = parts.next()?;
    if parts.next()? != slate.episode_id {
        return None;
    }
    match action {
        "approve" => Some(ApprovalDecision::Approved {
            approver: from,
            timestamp: Utc::now(),
            notes: None,
        }),
        "reject" => Some(ApprovalDecision::Rejected {
            reason: format!("Rejected by {from} via Telegram"),
            timestamp: Utc::now(),
        }),
        "research" => Some(ApprovalDecision::ResearchRequested {
            topic: slate.episode_id.clone(),
            notes: format!("More research requested by {from}"),
        }),
        "replace" => {
            let index: usize = parts.next()?.parse().ok()?;
            let story = slate.stories.get(index)?;
            Some(ApprovalDecision::ReplaceRequested {
                story_id: story.story_id.clone(),
                reason: format!("Swap requested by {from}"),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slate() -> EpisodeSlateProposal {
        EpisodeSlateProposal {
            episode_id: "EP-004".into(),
            episode_number: 4,
            target_duration_mins: 22.0,
            stories: vec![ProposedNewsStory {
                story_id: "a".into(),
                category: "Anime".into(),
                headline: "Studio <Trigger> & friends".into(),
                factual_summary: "x".into(),
                source_url: "https://a.test/?q=1&r=2".into(),
                sources: vec!["ANN".into()],
                announcement_date: "2026-10-17".into(),
                estimated_duration_mins: 3.0,
                verified: true,
            }],
            bulletins: vec![],
        }
    }

    #[test]
    fn html_is_escaped() {
        let html = TelegramApprovalGate::format_slate_html(&slate());
        assert!(html.contains("Studio &lt;Trigger&gt; &amp; friends"));
        assert!(html.contains("q=1&amp;r=2"));
    }

    #[test]
    fn callbacks_are_scoped_to_episode() {
        let cb = serde_json::json!({"from": {"username": "eli"}});
        assert!(matches!(parse_callback("approve:EP-004", &slate(), &cb), Some(ApprovalDecision::Approved { .. })));
        assert!(parse_callback("approve:EP-003", &slate(), &cb).is_none());
        assert_eq!(
            parse_callback("replace:EP-004:0", &slate(), &cb),
            Some(ApprovalDecision::ReplaceRequested { story_id: "a".into(), reason: "Swap requested by eli".into() })
        );
        assert!(parse_callback("replace:EP-004:9", &slate(), &cb).is_none());
    }
}
