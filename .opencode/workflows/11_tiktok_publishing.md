# TikTok Educational Short Publishing Pipeline (@focogaming)

## Objective
Publishes high-retention educational whiteboard Shorts directly to TikTok handle `@focogaming` using Upload-Post (MCP / REST API) with mandatory AI disclosures, cryptographic media deduplication, and automated status polling.

## Pipeline Steps
1. **RENDER VERTICAL MASTER**
   - Render Grease Pencil whiteboard frames in 9:16 vertical orientation (`1080x1920`).
   - Synthesize narration with Kokoro TTS and assemble with FFmpeg:
     ```bash
     atlas produce --topic <id> --format short
     ```
   - Target length: 30 to 60 seconds (fast-paced educational hook, visual morphs, conclusion).

2. **PREFLIGHT VETTING & DISCLOSURE**
   - Verify script citations, factual accuracy, and b-roll attribution.
   - Enforce mandatory synthetic media disclosure tag:
     `tiktok_is_ai_generated: true` (API) / `tiktokIsAigc: true` (MCP).

3. **CADENCE & DEDUPLICATION GATE**
   - Calculate SHA-256 media fingerprint.
   - Query `PublishLedger` to ensure the exact video has not already been uploaded to TikTok.
   - Verify rolling 24-hour publication count is below the platform cap (max 15 posts/24h).

4. **PUBLISH VIA UPLOAD-POST**
   - **Target Profile:** `focogaming`
   - **Endpoint:** `POST https://api.upload-post.com/api/upload` (or MCP tool `upload_video`)
   - **Command:**
     ```bash
     atlas publish --topic <id> --format short --platform tiktok --tiktok-user focogaming --privacy public
     ```
   - Or standalone reference script:
     ```bash
     py scripts/atlas_tiktok_publish.py --video exports/<id>/short/final.mp4 --metadata exports/<id>/short/youtube_metadata.json --user focogaming --privacy PUBLIC_TO_EVERYONE --out exports/<id>/short/tiktok_receipt.json
     ```

5. **POLL & RECONCILE RECEIPT**
   - Track asynchronous processing using `request_id`.
   - Poll `GET https://api.upload-post.com/api/uploadposts/upload-status/<request_id>` with exponential backoff until `completed`.
   - Capture published TikTok `post_id` and verified URL (`https://www.tiktok.com/@focogaming/video/<post_id>`).
   - Record immutable receipt and hash in `data/publish/ledger.json`.

## Rules
- Never retry an upload blindly without querying the existing `request_id`.
- Never post AI-generated whiteboard animations without setting `tiktok_is_ai_generated: true`.
- If testing or quality gate is uncertain, use `--tiktok-draft` (`post_mode: MEDIA_UPLOAD`) so the clip lands safely in the `@focogaming` TikTok inbox for manual review.
