use crate::news_desk::VerifiedNewsItem;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactCheckReport {
    pub total_candidates: usize,
    pub passed_stories: Vec<VerifiedNewsItem>,
    pub rejected_stories: Vec<(VerifiedNewsItem, String)>,
    pub estimated_total_minutes: f64,
}

pub struct NewsFactChecker {
    min_sources: usize,
    require_official_entity: bool,
}

impl Default for NewsFactChecker {
    fn default() -> Self {
        Self {
            min_sources: 1, // At least primary official or secondary corroboration
            require_official_entity: true,
        }
    }
}

impl NewsFactChecker {
    pub fn new(min_sources: usize, require_official_entity: bool) -> Self {
        Self {
            min_sources,
            require_official_entity,
        }
    }

    /// Verifies candidate stories against two-source confirmation, unverified rumors,
    /// and suppresses duplicate reporting or unverified speculation.
    pub fn verify_candidates(&self, candidates: Vec<VerifiedNewsItem>) -> FactCheckReport {
        let mut passed_stories = Vec::new();
        let mut rejected_stories = Vec::new();
        let mut seen_titles = HashSet::new();
        let total_candidates = candidates.len();

        for item in candidates {
            // Check for duplicate or near-duplicate
            let normalized_title: String = item
                .title
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();

            if seen_titles.contains(&normalized_title) {
                rejected_stories.push((item, "Duplicate story detected in wire".into()));
                continue;
            }

            // Reject unverified rumors
            if item.is_rumor || !item.is_verified {
                rejected_stories.push((item, "Story is marked as rumor or unverified speculation".into()));
                continue;
            }

            // Check official source presence
            if self.require_official_entity && item.studio_or_organization.trim().is_empty() {
                rejected_stories.push((item, "Missing official studio/organization attribution".into()));
                continue;
            }

            // Check primary source valid URL
            if !is_http_url(&item.primary_source_url) {
                rejected_stories.push((item, "Invalid primary source URL".into()));
                continue;
            }

            let source_count = 1 + item
                .secondary_source_url
                .as_deref()
                .filter(|url| is_http_url(url))
                .map_or(0, |_| 1);
            if source_count < self.min_sources {
                let reason = format!(
                    "Only {} verifiable source(s); {} required",
                    source_count, self.min_sources
                );
                rejected_stories.push((item, reason));
                continue;
            }

            seen_titles.insert(normalized_title);
            passed_stories.push(item);
        }

        let estimated_total_minutes = passed_stories
            .iter()
            .map(|s| s.estimated_duration_mins)
            .sum::<f64>()
            + 3.0; // Intro cold open (1m) + outro/credits (2m)

        FactCheckReport {
            total_candidates,
            passed_stories,
            rejected_stories,
            estimated_total_minutes,
        }
    }
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news_desk::AnimationCategory;

    #[test]
    fn test_fact_check_filtering() {
        let checker = NewsFactChecker::default();
        let stories = vec![
            VerifiedNewsItem {
                id: "1".into(),
                title: "Real News".into(),
                category: AnimationCategory::Anime,
                primary_source_url: "https://studio.com".into(),
                secondary_source_url: None,
                announcement_date: "2026-10-17".into(),
                studio_or_organization: "Studio X".into(),
                factual_summary: "Confirmed".into(),
                editorial_angle: "Exciting".into(),
                is_verified: true,
                is_rumor: false,
                estimated_duration_mins: 3.0,
            },
            VerifiedNewsItem {
                id: "2".into(),
                title: "Rumor Post".into(),
                category: AnimationCategory::Anime,
                primary_source_url: "https://reddit.com".into(),
                secondary_source_url: None,
                announcement_date: "2026-10-17".into(),
                studio_or_organization: "Unknown".into(),
                factual_summary: "Rumor only".into(),
                editorial_angle: "Speculation".into(),
                is_verified: false,
                is_rumor: true,
                estimated_duration_mins: 2.0,
            },
        ];

        let report = checker.verify_candidates(stories);
        assert_eq!(report.passed_stories.len(), 1);
        assert_eq!(report.rejected_stories.len(), 1);
    }

    #[test]
    fn test_two_source_requirement() {
        let checker = NewsFactChecker::new(2, true);
        let mut single = VerifiedNewsItem {
            id: "a".into(),
            title: "Single Source".into(),
            category: AnimationCategory::Anime,
            primary_source_url: "https://studio.com".into(),
            secondary_source_url: None,
            announcement_date: "2026-10-17".into(),
            studio_or_organization: "Studio X".into(),
            factual_summary: "Confirmed".into(),
            editorial_angle: "Exciting".into(),
            is_verified: true,
            is_rumor: false,
            estimated_duration_mins: 3.0,
        };
        let mut corroborated = single.clone();
        corroborated.id = "b".into();
        corroborated.title = "Corroborated".into();
        corroborated.secondary_source_url = Some("https://press.example.com".into());
        single.secondary_source_url = Some("not-a-url".into());

        let report = checker.verify_candidates(vec![single, corroborated]);
        assert_eq!(report.passed_stories.len(), 1);
        assert_eq!(report.passed_stories[0].id, "b");
    }
}
