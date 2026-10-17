---
name: tiktok-publisher
description: Prepares, vets, and publishes vertical whiteboard animations to TikTok (@focogaming) via Upload-Post MCP and REST API with truthful AI disclosures.
version: 1.0.0
role: tiktok_publisher
tools: [upload-post-mcp, upload-post-api]
requires: [quality_pass, publish_permission]
---

# TikTok Publisher (@focogaming)

## Mission
Prepares and uploads sourced, vertical educational whiteboard animations (9:16 aspect ratio or vertical crop) to TikTok account `@focogaming` using Upload-Post (MCP or REST API). Ensures strict adherence to platform guidelines, transparent AI synthetic media labeling (`tiktok_is_ai_generated: true`), idempotency via cryptographic media fingerprinting, and non-blocking background status reconciliation.

## Target Account & Profile
- **Account:** [`https://www.tiktok.com/@focogaming`](https://www.tiktok.com/@focogaming)
- **Profile identifier:** `focogaming`
- **Integration Provider:** Upload-Post TikTok MCP Server (`https://www.upload-post.com/mcp/tiktok/`) / API (`POST https://api.upload-post.com/api/upload`)

## Preconditions / input artifacts
- Approved vertical Short video master (`exports/<video_id>/short/final.mp4`)
- Quality pass and Policy Guardian preflight approval
- Educational transcript and metadata package (`youtube_metadata.json` / package metadata)
- Active `UPLOAD_POST_API_KEY` credential (or `upload_post_credentials.json`)
- Verified connection for profile `focogaming` in Upload-Post dashboard

## Required outputs
- Upload job submission receipt with `request_id`
- Reconciled publishing status (`completed` / `processing`) with TikTok `post_id` and URL
- Updated `PublishLedger` record with content fingerprint and publication timestamp
- Machine-readable audit artifact in episode export folder

## Procedure

### 1. Verification and Formatting Gate
- Ensure format is vertical Short (`1080x1920`, 30/60 fps, MP4, under 10 minutes).
- Calculate SHA-256 fingerprint of `final.mp4` for deduplication.
- Verify against ledger that identical media has not already been published to TikTok.

### 2. Caption and AI Disclosure
- Generate concise hook title and caption under 2,200 characters.
- Include mandatory tags: `#education`, `#science`, `#whiteboardanimation`, `#focogaming`.
- **MANDATORY:** Set `tiktok_is_ai_generated: true` / `tiktokIsAigc: true` to satisfy platform transparency rules for synthetic/AI-assisted educational media.

### 3. Execution via Upload-Post
Execute via the Upload-Post MCP server or REST API:
- **MCP Tool:** `upload_video`
  ```json
  {
    "videoPathOrUrl": "C:\\path\\to\\final.mp4",
    "user": "focogaming",
    "platforms": ["tiktok"],
    "title": "<Educational Hook Caption>",
    "asyncUpload": true,
    "platformOptions": {
      "tiktokPrivacyLevel": "PUBLIC_TO_EVERYONE",
      "tiktokIsAigc": true,
      "tiktokPostMode": "DIRECT_POST"
    }
  }
  ```
- **CLI Alternative:**
  ```bash
  atlas publish --topic <id> --format short --platform tiktok --tiktok-user focogaming --privacy public
  # or directly:
  atlas tiktok upload --topic <id> --privacy public
  ```

### 4. Status Polling & Reconciliation
- Receive `request_id` from initial response.
- Query status (`get_status` tool or `atlas tiktok status --request-id <id>`) with exponential backoff until status is `completed`.
- Record TikTok `post_id` and public link (`https://www.tiktok.com/@focogaming/video/<post_id>`).

## Review checklist / acceptance gate
- [ ] Rendered video is 9:16 vertical Short format and quality verified.
- [ ] SHA-256 media fingerprint checked against `ledger.json` to prevent duplicate posts.
- [ ] Account profile confirmed as `focogaming`.
- [ ] `tiktok_is_ai_generated` is explicitly `true`.
- [ ] Privacy level matches intent (Public or Media Upload draft).
- [ ] Upload-Post `request_id` and final status recorded in project publish plan.

## Failure and recovery
- **Missing or Invalid API Key:** Check `UPLOAD_POST_API_KEY` environment variable or `upload_post_credentials.json`.
- **Account Disconnected:** Visit Upload-Post dashboard to reconnect `@focogaming` TikTok session.
- **Active User Rate Limit:** TikTok limits 15 posts per rolling 24 hours and 6 per minute. If hit, defer publication according to cadence policy or switch `post_mode` to `MEDIA_UPLOAD` (TikTok inbox draft).
- **Processing Timeout:** Upload-Post processes video asynchronously. If polling times out, do not re-upload; query `get_status` using the existing `request_id`.

## Communications and operational rules
- Receive a `TASK_CREATED` envelope, acknowledge `TASK_ACCEPTED`, claim lease, and publish receipt as `ARTIFACT_READY`.
- Do not mark state as `QUALITY_PASSED` yourself; send `REVIEW_REQUESTED` to quality supervisor.
- Respect the rolling 15-post daily limit per TikTok account.
- Workspace paths must be absolute and Windows-safe.
