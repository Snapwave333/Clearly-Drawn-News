use crate::news_desk::{
    NewsStory, StorySource, WireItem, classify, detect_organization, is_non_news, is_rumor,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectedItem {
    pub outlet: String,
    pub title: String,
    pub url: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactCheckReport {
    pub total_candidates: usize,
    pub passed_stories: Vec<NewsStory>,
    pub bulletins: Vec<NewsStory>,
    pub rejected: Vec<RejectedItem>,
}

pub struct NewsFactChecker {
    min_sources: usize,
    require_official_entity: bool,
}

impl Default for NewsFactChecker {
    fn default() -> Self {
        Self {
            min_sources: 2,
            require_official_entity: true,
        }
    }
}

impl NewsFactChecker {
    pub fn new(min_sources: usize, require_official_entity: bool) -> Self {
        Self {
            min_sources: min_sources.max(1),
            require_official_entity,
        }
    }

    pub fn verify(&self, items: Vec<WireItem>) -> FactCheckReport {
        let total_candidates = items.len();
        let mut rejected = Vec::new();
        let mut seen_urls = HashSet::new();
        let mut eligible = Vec::new();
        for item in items {
            if !item.url.starts_with("http://") && !item.url.starts_with("https://") {
                rejected.push(reject(&item, "Source link is not a web URL"));
            } else if !seen_urls.insert(item.url.clone()) {
                rejected.push(reject(&item, "Duplicate wire entry"));
            } else if is_rumor(&item) {
                rejected.push(reject(&item, "Rumor, leak or unconfirmed report"));
            } else if is_non_news(&item) {
                rejected.push(reject(&item, "Review, column or promotional item, not news"));
            } else {
                eligible.push(item);
            }
        }

        let clusters = cluster(eligible);
        let mut passed_stories = Vec::new();
        let mut bulletins = Vec::new();
        for group in clusters {
            let story = build_story(&group);
            let has_org = !story.studio_or_organization.is_empty();
            if story.corroboration() >= self.min_sources && (has_org || !self.require_official_entity) {
                passed_stories.push(story);
            } else {
                bulletins.push(story);
            }
        }

        let rank = |a: &NewsStory, b: &NewsStory| {
            b.corroboration()
                .cmp(&a.corroboration())
                .then_with(|| b.published.cmp(&a.published))
        };
        passed_stories.sort_by(rank);
        bulletins.sort_by(rank);

        FactCheckReport {
            total_candidates,
            passed_stories,
            bulletins,
            rejected,
        }
    }
}

fn reject(item: &WireItem, reason: &str) -> RejectedItem {
    RejectedItem {
        outlet: item.outlet.clone(),
        title: item.title.clone(),
        url: item.url.clone(),
        reason: reason.into(),
    }
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "into", "its", "new", "will", "has",
    "have", "are", "was", "season", "anime", "film", "movie", "series", "show", "first", "gets",
    "reveals", "revealed", "announces", "announced", "official", "trailer", "teaser", "visual",
    "key", "cast", "more", "after", "about", "over", "out", "now", "how", "why", "what", "who",
    "set", "sets", "release", "date", "dates", "dated", "premiere", "premieres", "window", "confirms",
    "confirmed", "renews", "renewed", "greenlights", "greenlit", "opens", "opened", "adds", "names",
    "debuts", "unveils", "launches", "production", "studio", "studios", "animation", "animated",
];

pub fn title_tokens(title: &str) -> HashSet<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

pub fn same_story(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    let shared = a.intersection(b).count();
    let smaller = a.len().min(b.len());
    smaller > 0 && shared >= 2 && shared * 2 >= smaller
}

fn cluster(items: Vec<WireItem>) -> Vec<Vec<WireItem>> {
    let mut groups: Vec<(HashSet<String>, Vec<WireItem>)> = Vec::new();
    for item in items {
        let tokens = title_tokens(&item.title);
        let found = groups.iter_mut().find(|(group_tokens, members)| {
            same_story(group_tokens, &tokens) && !members.iter().any(|m| m.outlet == item.outlet)
        });
        match found {
            Some((group_tokens, members)) => {
                group_tokens.extend(tokens);
                members.push(item);
            }
            None => groups.push((tokens, vec![item])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

fn build_story(group: &[WireItem]) -> NewsStory {
    let lead = group
        .iter()
        .max_by_key(|item| item.summary.len())
        .unwrap_or(&group[0]);
    let combined: String = group
        .iter()
        .map(|i| format!("{} {}", i.title, i.summary))
        .collect::<Vec<_>>()
        .join(" ");
    let studio_or_organization = detect_organization(&combined).unwrap_or_default();
    let published = group.iter().filter_map(|i| i.published).min();
    let id_basis = title_tokens(&group[0].title)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    NewsStory {
        id: if id_basis.is_empty() { group[0].id.clone() } else { id_basis },
        title: group[0].title.clone(),
        category: classify(lead),
        studio_or_organization,
        summary: lead.summary.clone(),
        published,
        sources: group
            .iter()
            .map(|i| StorySource {
                outlet: i.outlet.clone(),
                title: i.title.clone(),
                url: i.url.clone(),
                published: i.published,
            })
            .collect(),
        article_text: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(outlet: &str, title: &str, url: &str) -> WireItem {
        WireItem {
            id: url.into(),
            outlet: outlet.into(),
            title: title.into(),
            url: url.into(),
            published: None,
            summary: format!("{title}. Confirmed by the studio."),
            feed_categories: vec![],
            anime_focused_outlet: false,
        }
    }

    #[test]
    fn corroborates_across_outlets_and_rejects_rumors() {
        let checker = NewsFactChecker::default();
        let report = checker.verify(vec![
            item("ANN", "MAPPA Sets Chainsaw Man Season 2 Premiere for July", "https://ann.test/1"),
            item("Brew", "Chainsaw Man Season 2 Premiere Set for July by MAPPA", "https://brew.test/1"),
            item("Brew", "Rumor: Pixar leak teases Toy Story 6", "https://brew.test/2"),
            item("ANN", "Toei Animation Opens New Studio in Manila", "https://ann.test/2"),
            item("ANN", "Episode Review: Frieren", "https://ann.test/review/3"),
        ]);
        assert_eq!(report.passed_stories.len(), 1);
        assert_eq!(report.passed_stories[0].corroboration(), 2);
        assert_eq!(report.passed_stories[0].studio_or_organization, "MAPPA");
        assert_eq!(report.bulletins.len(), 1);
        assert_eq!(report.rejected.len(), 2);
    }

    #[test]
    fn matches_rephrased_headlines_without_merging_different_titles() {
        let a = title_tokens("Pixar Sets Release Date for Elio Follow-Up Hoppers 2");
        let b = title_tokens("Pixar Hoppers 2 Release Date Set for March 2028");
        let c = title_tokens("Pixar Sets Release Date for Toy Story 5");
        assert!(same_story(&a, &b));
        assert!(!same_story(&a, &c));
    }

    #[test]
    fn same_outlet_never_corroborates_itself() {
        let checker = NewsFactChecker::default();
        let report = checker.verify(vec![
            item("ANN", "Pixar Dates Hoppers Sequel", "https://ann.test/a"),
            item("ANN", "Pixar Dates Hoppers Sequel for 2028", "https://ann.test/b"),
        ]);
        assert!(report.passed_stories.is_empty());
        assert_eq!(report.bulletins.len(), 2);
    }

    #[test]
    fn single_source_policy_accepts_attributed_items() {
        let checker = NewsFactChecker::new(1, true);
        let report = checker.verify(vec![item("ANN", "Pixar Dates Hoppers Sequel", "https://ann.test/a")]);
        assert_eq!(report.passed_stories.len(), 1);
    }
}
