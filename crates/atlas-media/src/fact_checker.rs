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
            if source_count < self.min_sources && !qualifies_as_official_primary(&item) {
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

/// The two-source rule, made explicit:
///
/// * Two independent sources always qualify (two outlets, or an official account plus one
///   independent outlet). An organisation's own channels and newsroom count as ONE source.
/// * A single source qualifies only as a primary source: a pinned, verified official account
///   or newsroom of the organisation, announcing that organisation's own project. The story is
///   then attributed to that organisation and credited on air as its own announcement.
/// * Anything else with one source (one outlet alone, or a third party's post) fails.
pub fn qualifies_as_official_primary(item: &VerifiedNewsItem) -> bool {
    item.sourcing.official_primary.is_some()
        && !item.studio_or_organization.trim().is_empty()
        && is_http_url(&item.primary_source_url)
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
                sourcing: Default::default(),
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
                sourcing: Default::default(),
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
            sourcing: Default::default(),
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

    fn official_item(official: bool) -> VerifiedNewsItem {
        VerifiedNewsItem {
            id: "pixar_trailer".into(),
            title: "Toy Story 5 | Official Trailer".into(),
            category: AnimationCategory::UpcomingReleases,
            primary_source_url: "https://www.youtube.com/watch?v=abc".into(),
            secondary_source_url: None,
            announcement_date: "2026-10-16".into(),
            studio_or_organization: "Pixar".into(),
            factual_summary: "Pixar released the official trailer.".into(),
            editorial_angle: String::new(),
            is_verified: true,
            is_rumor: false,
            estimated_duration_mins: 2.5,
            sourcing: crate::news_desk::Sourcing {
                independent_sources: 1,
                official_primary: official.then(|| "Pixar announced it on its official YouTube channel".to_string()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn two_source_rule_accepts_a_verified_studio_announcing_its_own_project() {
        let checker = NewsFactChecker::new(2, true);
        // Verified official account, own project: qualifies on its own as a primary source.
        let report = checker.verify_candidates(vec![official_item(true)]);
        assert_eq!(report.passed_stories.len(), 1);
        // The same single source without official status fails the two-source rule.
        let report = checker.verify_candidates(vec![official_item(false)]);
        assert_eq!(report.passed_stories.len(), 0);
        assert!(report.rejected_stories[0].1.contains("2 required"));
        // Official status never rescues an unattributed or rumoured story.
        let mut unattributed = official_item(true);
        unattributed.studio_or_organization.clear();
        let mut rumor = official_item(true);
        rumor.is_rumor = true;
        assert!(checker.verify_candidates(vec![unattributed, rumor]).passed_stories.is_empty());
        // Official post plus one independent outlet: two sources.
        let mut both = official_item(false);
        both.secondary_source_url = Some("https://www.cartoonbrew.com/x".into());
        assert_eq!(checker.verify_candidates(vec![both]).passed_stories.len(), 1);
    }
}
