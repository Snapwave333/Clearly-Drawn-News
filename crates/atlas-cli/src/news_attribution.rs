//! Studio/organisation attribution for live-wire stories the feed-text matcher could not place.
//! The director (Anthropic) reads only the story's own feed text and must quote the sentence that
//! names the organisation; the quote is checked against the text, so nothing is ever invented.
//! Stories still without a quoted organisation fail fact-checking as before.
use atlas_media::news_wire::WireStory;
use atlas_storyboard::DirectorClient;
use serde::Deserialize;

const MAX_PARALLEL: usize = 6;

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
        .to_lowercase()
}

/// Accepts an extraction only when the quoted sentence is in the source text and names the org.
pub(crate) fn verified_attribution(source_text: &str, organization: &str, sentence: &str) -> Option<(String, String)> {
    let org = organization.trim().trim_matches('"');
    let quote = sentence.trim().trim_matches('"');
    let generic = ["the studio", "studio", "the company", "company", "unknown", "none", "n/a", "the network"];
    if org.chars().count() < 2 || org.chars().count() > 60 || generic.contains(&org.to_lowercase().as_str()) {
        return None;
    }
    if quote.chars().count() < 12 {
        return None;
    }
    let (text, quote_n, org_n) = (normalize(source_text), normalize(quote), normalize(org));
    (text.contains(&quote_n) && quote_n.contains(&org_n)).then(|| (org.to_string(), quote.to_string()))
}

#[derive(Deserialize)]
struct Extraction {
    organization: Option<String>,
    sentence: Option<String>,
}

fn prompt(story: &WireStory) -> String {
    let text: String = story.source_text.chars().take(3500).collect();
    format!(
        "Headline: {}\n\nArticle text from the outlets' feeds:\n\"\"\"\n{text}\n\"\"\"\n\n\
         Which studio, company, network, festival or organisation made, produced or announced this news, \
         according to THIS TEXT ONLY? Copy, character for character, the one sentence from the text that names it. \
         Do not use outside knowledge. If the text names no such organisation, return nulls.\n\
         Return JSON: {{\"organization\": \"name as written in the text\" or null, \"sentence\": \"exact sentence\" or null}}",
        story.item.title
    )
}

async fn extract(director: &DirectorClient, story: &WireStory) -> Option<(String, String)> {
    let value = director
        .complete_json("You extract attributions from news text. Return valid JSON only.", &prompt(story), 0.0)
        .await
        .ok()?;
    let found: Extraction = serde_json::from_value(value).ok()?;
    verified_attribution(&story.source_text, found.organization.as_deref()?, found.sentence.as_deref()?)
}

/// Fills in missing organisations with quoted, verified director extractions. Returns how many.
pub(crate) async fn attribute_missing(stories: &mut [WireStory]) -> usize {
    let Some(director) = DirectorClient::from_env() else {
        return 0;
    };
    let pending: Vec<usize> = stories
        .iter()
        .enumerate()
        .filter(|(_, s)| s.item.studio_or_organization.trim().is_empty() && !s.item.is_rumor)
        .map(|(i, _)| i)
        .collect();
    let mut filled = 0;
    for batch in pending.chunks(MAX_PARALLEL) {
        let mut tasks = tokio::task::JoinSet::new();
        for &index in batch {
            let director = director.clone();
            let story = stories[index].clone();
            tasks.spawn(async move { (index, extract(&director, &story).await) });
        }
        while let Some(joined) = tasks.join_next().await {
            if let Ok((index, Some((org, sentence)))) = joined {
                let story = &mut stories[index];
                story.item.studio_or_organization = org;
                story.attribution_evidence = Some(sentence);
                story.attribution_method = Some(format!("director extraction ({}), quote verified in feed text", director.model()));
                filled += 1;
            }
        }
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "Kaiju Decode: The Ring of Aidara anime reveals teaser. The series is animated at   Studio Kai and premieres in April.";

    #[test]
    fn quotes_must_come_from_the_text_and_name_the_org() {
        let ok = verified_attribution(TEXT, "Studio Kai", "The series is animated at Studio Kai and premieres in April.");
        assert_eq!(ok.unwrap().0, "Studio Kai");
        assert!(verified_attribution(TEXT, "MAPPA", "The series is animated at MAPPA and premieres in April.").is_none());
        assert!(verified_attribution(TEXT, "Toho", "The series is animated at Studio Kai and premieres in April.").is_none());
        assert!(verified_attribution(TEXT, "the studio", "The series is animated at Studio Kai and premieres in April.").is_none());
        assert!(verified_attribution(TEXT, "Studio Kai", "Studio Kai").is_none());
    }
}
