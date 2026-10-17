use super::*;
use atlas_media::fact_checker::FactCheckReport;
use atlas_media::news_desk::WireOrigin;
use atlas_media::news_wire::FeedStatus;
use std::collections::BTreeMap;

const MAX_RUNS: usize = 200;
const FAILING_STREAK: usize = 3;
const FEED_COOLDOWN_HOURS: i64 = 6;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct WireRun {
    pub(crate) at: chrono::DateTime<Utc>,
    pub(crate) command: String,
    pub(crate) origin: WireOrigin,
    pub(crate) feeds: Vec<FeedStatus>,
    pub(crate) candidates: usize,
    pub(crate) passed: usize,
    pub(crate) corroborated: usize,
    pub(crate) rejection_reasons: BTreeMap<String, usize>,
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct WireHealthLog {
    #[serde(default)]
    pub(crate) runs: Vec<WireRun>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct FeedHealth {
    pub(crate) source: String,
    pub(crate) runs: usize,
    pub(crate) successes: usize,
    pub(crate) failure_streak: usize,
    pub(crate) average_entries: f64,
    pub(crate) last_error: Option<String>,
    pub(crate) skipped: usize,
    pub(crate) last_attempt: Option<chrono::DateTime<Utc>>,
}

impl FeedHealth {
    pub(crate) fn is_failing(&self) -> bool {
        self.failure_streak >= FAILING_STREAK
    }

    pub(crate) fn retry_after(&self) -> Option<chrono::DateTime<Utc>> {
        self.is_failing()
            .then_some(self.last_attempt)
            .flatten()
            .map(|at| at + chrono::Duration::hours(FEED_COOLDOWN_HOURS))
    }
}

pub(crate) fn feeds_to_skip(news_dir: &Path, now: chrono::DateTime<Utc>) -> Vec<(String, chrono::DateTime<Utc>)> {
    WireHealthLog::load(news_dir)
        .feed_health(MAX_RUNS)
        .into_iter()
        .filter_map(|feed| feed.retry_after().filter(|retry| *retry > now).map(|retry| (feed.source, retry)))
        .collect()
}

fn log_path(news_dir: &Path) -> PathBuf {
    news_dir.join("wire_health.json")
}

fn reason_key(reason: &str) -> String {
    let mut key = String::with_capacity(reason.len());
    let mut in_number = false;
    for ch in reason.chars() {
        if ch.is_ascii_digit() {
            if !in_number {
                key.push('N');
            }
            in_number = true;
        } else {
            in_number = false;
            key.push(ch);
        }
    }
    key
}

impl WireHealthLog {
    pub(crate) fn load(news_dir: &Path) -> Self {
        std::fs::read(log_path(news_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, news_dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(news_dir)?;
        std::fs::write(log_path(news_dir), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    fn push(&mut self, run: WireRun) {
        self.runs.push(run);
        if self.runs.len() > MAX_RUNS {
            let excess = self.runs.len() - MAX_RUNS;
            self.runs.drain(..excess);
        }
    }

    pub(crate) fn feed_health(&self, window: usize) -> Vec<FeedHealth> {
        let recent = &self.runs[self.runs.len().saturating_sub(window)..];
        let mut sources = recent
            .iter()
            .flat_map(|run| run.feeds.iter().map(|feed| feed.source.clone()))
            .collect::<Vec<_>>();
        sources.sort();
        sources.dedup();
        sources
            .into_iter()
            .map(|source| {
                let all = recent
                    .iter()
                    .filter_map(|run| run.feeds.iter().find(|feed| feed.source == source).map(|feed| (run.at, feed)))
                    .collect::<Vec<_>>();
                let skipped = all.iter().filter(|(_, feed)| feed.skipped).count();
                let last_attempt = all.iter().rev().find(|(_, feed)| !feed.skipped).map(|(at, _)| *at);
                let history = all.iter().filter(|(_, feed)| !feed.skipped).map(|(_, feed)| *feed).collect::<Vec<_>>();
                let successes = history.iter().filter(|feed| feed.error.is_none()).count();
                let failure_streak = history.iter().rev().take_while(|feed| feed.error.is_some()).count();
                let entries = history.iter().filter(|feed| feed.error.is_none()).map(|feed| feed.entries).sum::<usize>();
                FeedHealth {
                    runs: history.len(),
                    successes,
                    failure_streak,
                    average_entries: if successes == 0 { 0.0 } else { entries as f64 / successes as f64 },
                    last_error: history.iter().rev().find_map(|feed| feed.error.clone()),
                    skipped,
                    last_attempt,
                    source,
                }
            })
            .collect()
    }
}

pub(crate) fn record(
    news_dir: &Path,
    command: &str,
    origin: WireOrigin,
    feeds: &[FeedStatus],
    report: &FactCheckReport,
) -> anyhow::Result<()> {
    let mut rejection_reasons = BTreeMap::new();
    for (_, reason) in &report.rejected_stories {
        *rejection_reasons.entry(reason_key(reason)).or_insert(0) += 1;
    }
    let mut log = WireHealthLog::load(news_dir);
    log.push(WireRun {
        at: Utc::now(),
        command: command.into(),
        origin,
        feeds: feeds.to_vec(),
        candidates: report.total_candidates,
        passed: report.passed_stories.len(),
        corroborated: report.passed_stories.iter().filter(|s| s.secondary_source_url.is_some()).count(),
        rejection_reasons,
    });
    log.save(news_dir)
}

pub(crate) fn print_health(news_dir: &Path, window: usize) {
    let log = WireHealthLog::load(news_dir);
    let recent = &log.runs[log.runs.len().saturating_sub(window)..];
    if recent.is_empty() {
        println!("\nNo wire runs recorded yet. Run `atlas news research` first.");
        return;
    }
    let live = recent.iter().filter(|run| run.origin == WireOrigin::LiveFeeds).count();
    let candidates = recent.iter().map(|run| run.candidates).sum::<usize>();
    let passed = recent.iter().map(|run| run.passed).sum::<usize>();
    let corroborated = recent.iter().map(|run| run.corroborated).sum::<usize>();
    println!("\nWire health over the last {} runs (since {}):", recent.len(), recent[0].at.format("%Y-%m-%d %H:%M UTC"));
    println!("  Live wire:       {}/{} runs ({} used the curated fallback)", live, recent.len(), recent.len() - live);
    println!(
        "  Fact-check:      {}/{} candidates passed ({:.0}%), {} confirmed by two outlets",
        passed,
        candidates,
        if candidates == 0 { 0.0 } else { passed as f64 * 100.0 / candidates as f64 },
        corroborated
    );
    let mut reasons = BTreeMap::new();
    for run in recent {
        for (reason, count) in &run.rejection_reasons {
            *reasons.entry(reason.clone()).or_insert(0) += count;
        }
    }
    for (reason, count) in &reasons {
        println!("    ✗ {count:>3} × {reason}");
    }
    println!("\n  Feeds:");
    let feeds = log.feed_health(window);
    for feed in &feeds {
        let marker = match feed.retry_after() {
            Some(retry) if retry > Utc::now() => format!("  ⚠ FAILING, paused until {}", retry.format("%H:%M UTC")),
            Some(_) => "  ⚠ FAILING, will retry next run".to_string(),
            None => String::new(),
        };
        println!(
            "    {:<20} {:>3}/{:<3} ok  avg {:>5.1} items  streak {:>2} failed  {:>2} skipped{}",
            feed.source, feed.successes, feed.runs, feed.average_entries, feed.failure_streak, feed.skipped, marker
        );
        if feed.is_failing()
            && let Some(error) = &feed.last_error
        {
            println!("      last error: {error}");
        }
    }
    let failing = feeds.iter().filter(|feed| feed.is_failing()).count();
    if failing > 0 {
        println!("\n  {failing} feed(s) failed {FAILING_STREAK}+ runs in a row; check network access or the feed URL.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(results: &[(&str, Option<usize>)]) -> WireRun {
        WireRun {
            at: Utc::now(),
            command: "research".into(),
            origin: WireOrigin::LiveFeeds,
            feeds: results
                .iter()
                .map(|(source, entries)| FeedStatus {
                    source: (*source).into(),
                    entries: entries.unwrap_or(0),
                    error: entries.is_none().then(|| format!("{source}: error sending request")),
                    skipped: false,
                })
                .collect(),
            candidates: 5,
            passed: 3,
            corroborated: 1,
            rejection_reasons: BTreeMap::new(),
        }
    }

    #[test]
    fn detects_feeds_failing_in_a_row() {
        let mut log = WireHealthLog::default();
        log.push(run(&[("ANN", Some(20)), ("Brew", Some(10))]));
        log.push(run(&[("ANN", Some(30)), ("Brew", None)]));
        log.push(run(&[("ANN", None), ("Brew", None)]));
        log.push(run(&[("ANN", Some(10)), ("Brew", None)]));
        let health = log.feed_health(10);
        let ann = health.iter().find(|f| f.source == "ANN").unwrap();
        let brew = health.iter().find(|f| f.source == "Brew").unwrap();
        assert_eq!((ann.successes, ann.runs, ann.failure_streak), (3, 4, 0));
        assert_eq!(ann.average_entries, 20.0);
        assert!(!ann.is_failing());
        assert_eq!(brew.failure_streak, 3);
        assert!(brew.is_failing());
        assert_eq!(brew.last_error.as_deref(), Some("Brew: error sending request"));
        assert_eq!(log.feed_health(1)[1].failure_streak, 1);
    }

    #[test]
    fn failing_feeds_pause_then_retry_without_counting_skips() {
        let mut log = WireHealthLog::default();
        let start = Utc::now() - chrono::Duration::hours(1);
        for hour in 0..3 {
            let mut r = run(&[("ANN", Some(5)), ("Brew", None)]);
            r.at = start + chrono::Duration::minutes(hour * 10);
            log.push(r);
        }
        let mut skipped = run(&[("ANN", Some(5))]);
        skipped.feeds.push(FeedStatus { source: "Brew".into(), entries: 0, error: None, skipped: true });
        log.push(skipped);

        let brew = log.feed_health(10).into_iter().find(|f| f.source == "Brew").unwrap();
        assert_eq!((brew.runs, brew.failure_streak, brew.skipped), (3, 3, 1));
        let retry = brew.retry_after().unwrap();
        assert_eq!(retry, start + chrono::Duration::minutes(20) + chrono::Duration::hours(FEED_COOLDOWN_HOURS));
        assert!(retry > Utc::now());
        let ann = log.feed_health(10).into_iter().find(|f| f.source == "ANN").unwrap();
        assert!(ann.retry_after().is_none());

        let mut recovered = run(&[("ANN", Some(5)), ("Brew", Some(7))]);
        recovered.at = Utc::now();
        log.push(recovered);
        let brew = log.feed_health(10).into_iter().find(|f| f.source == "Brew").unwrap();
        assert_eq!(brew.failure_streak, 0);
        assert!(brew.retry_after().is_none());
    }

    #[test]
    fn log_is_capped_and_reasons_grouped() {
        let mut log = WireHealthLog::default();
        for _ in 0..(MAX_RUNS + 5) {
            log.push(run(&[("ANN", Some(1))]));
        }
        assert_eq!(log.runs.len(), MAX_RUNS);
        assert_eq!(reason_key("Only 1 verifiable source(s); 12 required"), "Only N verifiable source(s); N required");
        assert_eq!(reason_key("Duplicate story detected in wire"), "Duplicate story detected in wire");
    }
}
