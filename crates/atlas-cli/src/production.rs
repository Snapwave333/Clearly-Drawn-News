use super::*;

pub(crate) struct StageTimer {
    started: std::time::Instant,
    current: Option<(&'static str, std::time::Instant)>,
    completed: Vec<StageTiming>,
}

#[derive(Serialize)]
pub(crate) struct StageTiming {
    pub(crate) stage: &'static str,
    pub(crate) seconds: f64,
}

#[derive(Serialize)]
pub(crate) struct StageTimingReport {
    pub(crate) recorded_at: chrono::DateTime<Utc>,
    pub(crate) total_seconds: f64,
    pub(crate) slowest: Option<String>,
    pub(crate) stages: Vec<StageTiming>,
}

impl StageTimer {
    pub(crate) fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            current: None,
            completed: Vec::new(),
        }
    }

    pub(crate) fn begin(&mut self, stage: &'static str) {
        self.end_current();
        self.current = Some((stage, std::time::Instant::now()));
    }

    pub(crate) fn end_current(&mut self) {
        if let Some((stage, started)) = self.current.take() {
            self.completed.push(StageTiming {
                stage,
                seconds: started.elapsed().as_secs_f64(),
            });
        }
    }

    pub(crate) fn finish(mut self) -> StageTimingReport {
        self.end_current();
        let slowest = self
            .completed
            .iter()
            .max_by(|a, b| a.seconds.total_cmp(&b.seconds))
            .map(|timing| timing.stage.to_string());
        StageTimingReport {
            recorded_at: Utc::now(),
            total_seconds: self.started.elapsed().as_secs_f64(),
            slowest,
            stages: self.completed,
        }
    }
}

pub(crate) fn env_parallelism(name: &str) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

pub(crate) fn validate_format_duration(format: VideoFormat, seconds: f64) -> anyhow::Result<()> {
    match format {
        VideoFormat::Short if seconds > 60.0 => anyhow::bail!(
            "Shorts QA rejected {seconds:.2}s output; maximum is 60 seconds"
        ),
        VideoFormat::Short => Ok(()),
        VideoFormat::Long => atlas_core::EpisodeDurationPolicy::default()
            .validate(seconds)
            .map_err(|error| anyhow::anyhow!("Long-form QA rejected output: {error}")),
    }
}

pub(crate) async fn render_shots(
    pipeline: &BlenderPipeline,
    jobs: Vec<atlas_blender::RenderJob>,
    parallelism: usize,
) -> anyhow::Result<()> {
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(parallelism.max(1)));
    let mut tasks = tokio::task::JoinSet::new();
    for job in jobs {
        let pipeline = pipeline.clone();
        let permits = permits.clone();
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await?;
            println!("   Rendering {}...", job.shot_spec.shot_id);
            let output = pipeline.render(&job).await?;
            println!("   ✓ {} {} real frames", job.shot_spec.shot_id, output.frames.len());
            anyhow::Ok(())
        });
    }
    while let Some(joined) = tasks.join_next().await {
        if let Err(error) = joined? {
            tasks.abort_all();
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn generate_vector_diagram(term: &str, index: usize, output_dir: &Path) -> Option<PathBuf> {
    let term_hash = term
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| (acc ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    let svg_path = output_dir.join(format!("vector_{index}_{term_hash:016x}.svg"));
    if svg_path.is_file() {
        return Some(svg_path);
    }
    let escaped = term
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let mut svg_content = String::from(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 240" width="500" height="240">
  <defs>
    <linearGradient id="cardGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#0f172a" stop-opacity="0.95" />
      <stop offset="100%" stop-color="#1e293b" stop-opacity="0.95" />
    </linearGradient>
    <filter id="cardGlow" x="-10%" y="-10%" width="120%" height="120%">
      <feDropShadow dx="0" dy="4" stdDeviation="6" flood-color="#0284c7" flood-opacity="0.35"/>
    </filter>
  </defs>
  <rect x="15" y="15" width="470" height="210" rx="20" fill="url(#cardGrad)" stroke="#38bdf8" stroke-width="3" filter="url(#cardGlow)"/>
  <circle cx="55" cy="55" r="16" fill="#38bdf8"/>
"##);
    svg_content.push_str(&format!(
        r##"  <text x="55" y="61" font-family="Arial, Helvetica, sans-serif" font-size="14" font-weight="bold" fill="#0f172a" text-anchor="middle">{}</text>
  <text x="90" y="62" font-family="Arial, Helvetica, sans-serif" font-size="22" font-weight="bold" fill="#f8fafc">{}</text>
"##,
        index + 1,
        escaped
    ));
    svg_content.push_str(r##"  <line x1="40" y1="95" x2="460" y2="95" stroke="#334155" stroke-width="2"/>
  <rect x="40" y="125" width="12" height="12" rx="3" fill="#0284c7"/>
  <text x="65" y="136" font-family="Arial, Helvetica, sans-serif" font-size="15" fill="#94a3b8">Scientific core mechanism</text>
  <rect x="40" y="165" width="12" height="12" rx="3" fill="#38bdf8"/>
  <text x="65" y="176" font-family="Arial, Helvetica, sans-serif" font-size="15" fill="#94a3b8">System behavior &amp; state</text>
</svg>"##);

    std::fs::write(&svg_path, svg_content).ok()?;
    Some(svg_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_timer_reports_every_stage_and_slowest() {
        let mut timer = StageTimer::new();
        timer.begin("fast");
        timer.begin("slow");
        std::thread::sleep(std::time::Duration::from_millis(20));
        timer.begin("last");
        let report = timer.finish();
        let names = report.stages.iter().map(|stage| stage.stage).collect::<Vec<_>>();
        assert_eq!(names, ["fast", "slow", "last"]);
        assert_eq!(report.slowest.as_deref(), Some("slow"));
        assert!(report.total_seconds >= report.stages[1].seconds);
    }

    #[test]
    fn duration_qa_matches_format_limits() {
        assert!(validate_format_duration(VideoFormat::Short, 45.0).is_ok());
        assert!(validate_format_duration(VideoFormat::Short, 61.0).is_err());
        assert!(validate_format_duration(VideoFormat::Long, 600.0).is_err());
        assert!(validate_format_duration(VideoFormat::Long, 1320.0).is_ok());
        assert!(validate_format_duration(VideoFormat::Long, 1900.0).is_err());
    }

    #[test]
    fn vector_diagrams_are_unique_per_term() {
        let dir = std::env::temp_dir().join(format!("atlas_vectors_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = generate_vector_diagram("Photosynthesis", 0, &dir).unwrap();
        let second = generate_vector_diagram("Mitochondria", 0, &dir).unwrap();
        let repeat = generate_vector_diagram("Photosynthesis", 0, &dir).unwrap();
        assert_ne!(first, second);
        assert_eq!(first, repeat);
        assert!(std::fs::read_to_string(&second).unwrap().contains("Mitochondria"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
