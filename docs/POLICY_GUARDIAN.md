# Policy & Channel Guardian

## Purpose

The Policy & Channel Guardian is a mandatory pre-publish agent. ATLAS cannot upload a video until the guardian approves the complete package and the authenticated channel.

## What runs before every upload

1. Download the current official YouTube Community Guidelines, spam policy, channel monetization policy, altered-content policy, and upload-limit guidance.
2. Normalize and fingerprint every policy page.
3. Compare the current fingerprints with the last accepted baseline.
4. Inspect the title, description, full narration transcript, synthetic-media disclosure, captions, thumbnail, B-roll attribution, and duplicate-media fingerprint.
5. Query the authenticated YouTube channel through the YouTube Data API and verify that the channel exists, is linked, reports upload eligibility, and returns a privacy state.
6. Require a matching YouTube Studio browser review from the last 24 hours with no visible Community Guidelines warning or active strike.

Step 6 is per publish request (`ChannelPreflight.require_browser_review`). Explainer uploads (`atlas publish`) keep it on. Clearly Drawn News (`atlas news publish` and the daemon's news slots) turns it off so the twice-daily show can run unattended: the browser identity and freshness checks are skipped, and a recorded review is still enforced (warning or strike blocks) when one exists. Every other check above still runs for news.

Any failed check, unavailable official page, changed policy page, missing channel status, or high-risk content review blocks publication.

## Policy change workflow

The first successful live check establishes the baseline from official YouTube pages. Later changes are stored in `data/policy/pending_baseline.json` and block uploads.

Review the current official pages in a real browser, then accept the observed snapshot:

```text
atlas policy accept-current
```

The next publish attempt must fetch all official pages again and match the accepted snapshot.

## Reports

The latest report is stored at `data/policy/latest_report.json`. Per-video reports are stored under `data/policy/reports/`.

```text
atlas policy status
```

Refresh only the official policy sources without attempting an upload:

```text
atlas policy refresh
```

Every report records the exact source URLs, retrieval time, normalized SHA-256 fingerprints, individual checks, and the final publish decision.

## Fail-closed behavior

- Network or official-source failure blocks upload.
- A policy fingerprint change blocks upload.
- Missing AI or synthetic-media disclosure blocks upload.
- Missing captions or thumbnail blocks upload.
- Missing attribution for imported B-roll blocks upload.
- Duplicate media blocks upload.
- High-risk policy language requires review and blocks upload.
- Missing or ineligible channel state blocks upload.
- A stale or mismatched browser channel review blocks upload.
- An active strike blocks upload. A visible warning blocks upload unless its required training is complete and YouTube reports a future expiration date.

## Channel limitation

The YouTube Data API does not expose a complete Community Guidelines strike history. The guardian therefore requires a fresh review of the real authenticated YouTube Studio dashboard. A browser automation runner records the observation with `atlas policy record-channel-review`; it does not clear, dismiss, or reinterpret warnings.

## Official monitored sources

- https://support.google.com/youtube/answer/9288567
- https://support.google.com/youtube/answer/2801973
- https://support.google.com/youtube/answer/1311392
- https://support.google.com/youtube/answer/14328491
- https://support.google.com/youtube/answer/57407
