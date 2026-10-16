# Full analytics-to-creative-learning loop

1. **Acquire**: Through connected YouTube owner OAuth, query video summaries (`views`, `estimatedMinutesWatched`, `averageViewDuration`, `averageViewPercentage`, subscription deltas), optional compatible traffic source reports, and per-video retention curves.
2. **Classify maturity**: Store upload timestamp, requested date window, access errors and sample availability. First retention data commonly takes 1–2 days; retry later rather than inserting zeros.
3. **Interpret**: Retention analyst plots `audienceWatchRatio` and `relativeRetentionPerformance` separately. Never clamp values above 1 or imply a single sample represents one individual viewer.
4. **Scene alignment**: Join normalized 100-point retention buckets to exact `scene_timeline.json` based on final published duration. Attach narrative function, narration, diagram type and transitions; include uncertainty and alternate causes.
5. **Context**: Evaluate raw views, watch time, video age, cohort, traffic source availability and episode topic; locate causes outside animation before proposing improvements.
6. **Hypothesize**: Experiment Designer records one variable change for **future videos**—for example shorter introduction context or clearer staged diagram—without retroactively editing published footage to manipulate results.
7. **Run**: Pre-register observations on comparable episodes, independently approve a limited roll-out, and report factual/creative quality separately.
8. **Decide**: Cohort Evaluator calls out confounders and labels the result observational unless there is a valid randomized experiment; auditor checks no harmful drift.
9. **Improve**: Recursive Optimizer proposes a patch to a versioned creative config. Promote only after review; preserve baseline, holdout and rollback.
10. **Repeat**: Check next comparable video cohort and reverse unsuccessful changes. Balance experimentation with unique art direction to avoid template stagnation.

## Counterexamples
- One video has a spike at 61 seconds → **do not conclude** `the animated arrow caused retention gains`.
- One video loses viewers at a long scientific derivation → **do not delete all math**; test stronger prerequisite setup and slower staging.
- New viral topic draws more views but has low relative retention → **do not equate popularity with better teaching**.
- A static credits card drops retention at the end after the question is answered → do not automatically cut transparent AI credits.
- YouTube Studio shows a metric not exposed to the app's authorized API → label unavailable; do not scrape Studio or fabricate matching data.
