# Retention interpretation — editorial use, not pseudo-science

**Data available:** Per-video official API audienceWatchRatio / relativeRetentionPerformance with 100 normalized time points when returned. Some other retention metrics may be supported; treat them as optional.

**Example:** a seven-minute video has a sample at ratio 0.30; this bucket ends near 126 seconds. It is not automatically a distinct, one-second drop. A scene that spans 115–138 seconds can be tagged for review but not proven responsible.

**Segments:** intro (0–~30s), teachable beats, proof/demo, payoff, chapter bridges, end credits. Map using published exact duration, not planned EDL. Retention is cumulative viewing behavior and may show >100% replayed segments.

**Possible explanations and corresponding edits** (hypotheses ONLY):
- Drop in first 15–30s: confusing premise, wrong audience expectation, insufficient specific demonstration. Test more concrete opening, not unsupported drama.
- Decline at busy diagram: split reveal, pause narration, remove labels, add a simple example.
- Spike/rewatch at a worked example: viewers may be interested **or confused**; audit comprehensibility before repeating it.
- Drop at sponsor-esque tangent or long aside: cut digression, return to central explanatory question.
- Stable attention during simple sketch morph: consider testing similar visual reasoning on another matched topic; avoid copying the art.
- Drop near resolution: may indicate audience understood and left; assess how much promise had already been fulfilled.

**Control confounders:** topic novelty, source traffic mix, video age, runtime, seasonality, publishing time, external event, small audience, Shorts versus longform, length normalization and survey-comparable episodes.

**No causal leap:** Observational retention curves cannot prove a scene caused an effect; propose next-video variants and watch comparable patterns. Do not use YouTube API data to create prohibited custom composite scoring unless your API project has authorization for those metrics.

**Cohort rules:** match audience, format, topic family, approximate length and observation window. Prepare single-variable trial, preregister outcome and revert rules, ask independent auditor before promoting.
