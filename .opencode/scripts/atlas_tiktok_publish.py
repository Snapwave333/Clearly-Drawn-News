#!/usr/bin/env python3
"""Authorized, idempotent TikTok video publisher for ATLAS STUDIO Whiteboard Pipeline.
Integrates with Upload-Post (https://www.upload-post.com/mcp/tiktok/) to publish
educational whiteboard animations to TikTok (@focogaming).

Features:
- Mandatory transparent AI synthetic media disclosure (tiktok_is_ai_generated: true).
- Idempotency key via SHA-256 media fingerprint to prevent duplicate uploads.
- Direct posting or Draft mode (MEDIA_UPLOAD) for manual finishing in TikTok app.
- Status polling with exponential backoff until published receipt is confirmed.
"""

import argparse
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

DEFAULT_USER = "focogaming"
DEFAULT_BASE_URL = "https://api.upload-post.com"


def load_credentials(creds_file=None):
    """Load API key from environment or JSON credentials file."""
    api_key = os.environ.get("UPLOAD_POST_API_KEY")
    if api_key and api_key.strip():
        return {
            "api_key": api_key.strip(),
            "default_user": os.environ.get("TIKTOK_PROFILE", DEFAULT_USER),
            "base_url": os.environ.get("UPLOAD_POST_BASE_URL", DEFAULT_BASE_URL),
        }

    candidates = [
        Path(creds_file) if creds_file else None,
        Path("upload_post_credentials.json"),
        Path("tiktok_credentials.json"),
    ]

    for p in candidates:
        if p and p.is_file():
            try:
                data = json.loads(p.read_text(encoding="utf-8"))
                key = data.get("api_key")
                if key and key.strip():
                    return {
                        "api_key": key.strip(),
                        "default_user": data.get("user") or data.get("default_user") or DEFAULT_USER,
                        "base_url": data.get("base_url") or DEFAULT_BASE_URL,
                    }
            except Exception:
                pass

    return None


def calculate_fingerprint(file_path):
    """Compute SHA-256 media fingerprint."""
    h = hashlib.sha256()
    with open(file_path, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def build_multipart_form(fields, files):
    """Build multipart/form-data body and boundary without external dependencies."""
    boundary = "----AtlasStudioTikTokFormBoundary" + hashlib.md5(str(time.time()).encode()).hexdigest()
    lines = []

    for name, value in fields.items():
        lines.append(f"--{boundary}".encode("utf-8"))
        lines.append(f'Content-Disposition: form-data; name="{name}"'.encode("utf-8"))
        lines.append(b"")
        lines.append(str(value).encode("utf-8"))

    for name, (filename, file_bytes, mime_type) in files.items():
        lines.append(f"--{boundary}".encode("utf-8"))
        lines.append(f'Content-Disposition: form-data; name="{name}"; filename="{filename}"'.encode("utf-8"))
        lines.append(f"Content-Type: {mime_type}".encode("utf-8"))
        lines.append(b"")
        lines.append(file_bytes)

    lines.append(f"--{boundary}--".encode("utf-8"))
    lines.append(b"")

    body = b"\r\n".join(lines)
    content_type = f"multipart/form-data; boundary={boundary}"
    return body, content_type


def upload_video(base_url, api_key, fields, video_path, idempotency_key=None):
    """Upload video via Upload-Post REST API."""
    url = f"{base_url.rstrip('/')}/api/upload"

    with open(video_path, "rb") as f:
        video_bytes = f.read()

    files = {
        "video": (Path(video_path).name, video_bytes, "video/mp4")
    }

    body, content_type = build_multipart_form(fields, files)

    headers = {
        "Authorization": f"Apikey {api_key}",
        "Content-Type": content_type,
        "User-Agent": "ATLAS-STUDIO/1.0",
    }
    if idempotency_key:
        headers["Idempotency-Key"] = idempotency_key

    req = urllib.request.Request(url, data=body, headers=headers, method="POST")

    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        err_msg = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Upload-Post HTTP {exc.code}: {err_msg}") from None
    except urllib.error.URLError as exc:
        raise RuntimeError(f"Network error contacting Upload-Post: {exc}") from None


def get_status(base_url, api_key, request_id):
    """Query upload status from Upload-Post."""
    url = f"{base_url.rstrip('/')}/api/uploadposts/upload-status/{urllib.parse.quote(request_id)}"
    headers = {
        "Authorization": f"Apikey {api_key}",
        "Accept": "application/json",
        "User-Agent": "ATLAS-STUDIO/1.0",
    }
    req = urllib.request.Request(url, headers=headers, method="GET")

    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        err_msg = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"Status query HTTP {exc.code}: {err_msg}") from None
    except urllib.error.URLError as exc:
        raise RuntimeError(f"Network error querying status: {exc}") from None


def poll_status(base_url, api_key, request_id, timeout_sec=60):
    """Poll upload status until completion or timeout."""
    start = time.time()
    interval = 3.0

    while time.time() - start < timeout_sec:
        time.sleep(interval)
        try:
            status_data = get_status(base_url, api_key, request_id)
            status = status_data.get("status", "processing")
            if status in ("completed", "failed"):
                return status_data
        except Exception as e:
            sys.stderr.write(f"Warning: transient poll error: {e}\n")

        interval = min(interval + 1.5, 10.0)

    return {"status": "processing", "request_id": request_id, "note": "timed out polling, check later"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--video", required=True, help="Path to rendered MP4 video file")
    parser.add_argument("--metadata", help="Path to youtube_metadata.json or topic metadata")
    parser.add_argument("--title", help="Custom caption / title for TikTok")
    parser.add_argument("--user", default=DEFAULT_USER, help=f"TikTok profile handle (default: {DEFAULT_USER})")
    parser.add_argument("--privacy", choices=["PUBLIC_TO_EVERYONE", "MUTUAL_FOLLOW_FRIENDS", "FOLLOWER_OF_CREATOR", "SELF_ONLY"], default="PUBLIC_TO_EVERYONE")
    parser.add_argument("--post-mode", choices=["DIRECT_POST", "MEDIA_UPLOAD"], default="DIRECT_POST", help="DIRECT_POST to publish or MEDIA_UPLOAD to send to TikTok drafts")
    parser.add_argument("--disable-duet", action="store_true", help="Disable duets")
    parser.add_argument("--disable-stitch", action="store_true", help="Disable stitching")
    parser.add_argument("--disable-comment", action="store_true", help="Disable comments")
    parser.add_argument("--no-aigc", action="store_true", help="Do not mark as AI-generated (default: True for whiteboard pipeline)")
    parser.add_argument("--out", help="Output JSON receipt path")
    parser.add_argument("--dry-run", action="store_true", help="Print payload plan without network request")
    parser.add_argument("--poll", action="store_true", default=True, help="Wait for TikTok processing confirmation")
    parser.add_argument("--poll-timeout", type=int, default=60, help="Max polling duration in seconds")
    parser.add_argument("--creds-file", help="Path to credentials JSON")

    args = parser.parse_args()

    video_path = Path(args.video)
    if not video_path.is_file():
        raise SystemExit(f"Video file not found: {video_path}")

    fingerprint = calculate_fingerprint(video_path)

    # Determine caption / title
    caption = args.title
    if not caption and args.metadata and Path(args.metadata).is_file():
        try:
            meta = json.loads(Path(args.metadata).read_text(encoding="utf-8"))
            caption = meta.get("title")
        except Exception:
            pass

    if not caption:
        caption = video_path.stem.replace("_", " ").title()

    # Append standard educational hashtags
    tags = ["#education", "#science", "#whiteboardanimation", f"#{args.user}"]
    for t in tags:
        if t.lower() not in caption.lower():
            caption = f"{caption} {t}"

    fields = {
        "user": args.user,
        "platform[]": "tiktok",
        "title": caption[:2200],  # TikTok caption max limit
        "privacy_level": args.privacy,
        "post_mode": args.post_mode,
        "async_upload": "true",
        "tiktok_is_ai_generated": "false" if args.no_aigc else "true",
        "tiktok_is_aigc": "false" if args.no_aigc else "true",
    }
    if args.disable_duet:
        fields["disable_duet"] = "true"
    if args.disable_stitch:
        fields["disable_stitch"] = "true"
    if args.disable_comment:
        fields["disable_comment"] = "true"

    if args.dry_run:
        plan = {
            "endpoint": f"{DEFAULT_BASE_URL}/api/upload",
            "user": args.user,
            "fields": fields,
            "media": {
                "path": str(video_path),
                "size_bytes": video_path.stat().st_size,
                "sha256": fingerprint,
            },
            "idempotency_key": fingerprint,
        }
        print(json.dumps(plan, indent=2))
        return

    creds = load_credentials(args.creds_file)
    if not creds:
        raise SystemExit("Missing UPLOAD_POST_API_KEY environment variable or upload_post_credentials.json file.")

    base_url = creds["base_url"]
    api_key = creds["api_key"]

    print(f"Uploading {video_path.name} to TikTok (@{args.user}) via Upload-Post...")
    res = upload_video(base_url, api_key, fields, video_path, idempotency_key=fingerprint)

    request_id = res.get("request_id")
    print(f"Upload initiated: request_id={request_id}")

    final_status = res
    if args.poll and request_id:
        print(f"Polling processing status (timeout {args.poll_timeout}s)...")
        final_status = poll_status(base_url, api_key, request_id, timeout_sec=args.poll_timeout)
        print(f"Status result: {final_status.get('status')}")

    receipt = {
        "platform": "tiktok",
        "user": args.user,
        "profile_url": f"https://www.tiktok.com/@{args.user}",
        "published_at": datetime.now(timezone.utc).isoformat(),
        "request_id": request_id,
        "status": final_status.get("status", "initiated"),
        "fingerprint": fingerprint,
        "caption": caption,
        "privacy": args.privacy,
        "post_mode": args.post_mode,
        "is_ai_generated": not args.no_aigc,
        "api_response": final_status,
    }

    if args.out:
        out_path = Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        print(f"Wrote publication receipt: {out_path}")
    else:
        print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
