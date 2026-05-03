# Roadmap

[English](ROADMAP.md) | [中文](ROADMAP_CN.md)

This document tracks planned product and engineering work for the current release line. Changelogs only describe shipped work; this file describes work that still needs implementation and validation.

## Current Position

Baidu Netdisk, Google Drive, and generic WebDAV/AList sync should not ship as a simple final step in the card offload pipeline. Cloud storage is slow, unstable, and retry-heavy, so it must be decoupled from local offload.

Release criteria:

- After local copy, verification, and MHL generation complete, the source card should be released as soon as possible.
- Cloud upload should read from a verified destination volume, not from the source card.
- Cloud upload needs an independent queue with resume, pause, cancel, retry, throttling, and concurrency controls.
- Every remote object needs an explicit integrity state: hash-verified, size-only verified, or requiring readback verification.
- Proxy-only upload must be a first-class feature for fast review handoff to editors, directors, DITs, or production.

## P0: Cloud Upload Queue

Goal: move cloud sync from an offload sub-step into an independent background queue.

Planned work:

- Add persistent upload entities: `cloud_upload_jobs`, `cloud_upload_items`, `cloud_upload_parts`, and `cloud_upload_events`.
- Upload jobs read from verified destinations, proxy caches, or report directories.
- After local offload verification passes, the source card can be unmounted or released.
- The upload worker supports pause, resume, cancel, retry, exponential backoff, and app restart recovery.
- The queue supports global throttling, per-provider concurrency, per-job concurrency, and disk read concurrency limits.
- The UI exposes an upload queue view with upload, verification, failure, and retry status per object.

Required behavior:

- Starting a local offload while a cloud upload is running must not be blocked.
- Starting a second cloud upload while one is running should enqueue or run according to configured concurrency.
- Cancelling an upload should stop at a part boundary and preserve recoverable state.
- Restarting the app should resume incomplete upload jobs.

## P0: Proxy-only Review Delivery

Goal: make "upload proxies only" an explicit delivery mode, not a side option inside full cloud backup.

Upload modes:

- `Proxy Only`: upload proxy files only for fast review and cross-team checking.
- `Proxy + Reports`: upload proxies, thumbnails, Rushes Log, and job reports.
- `MHL + Reports`: upload verification artifacts only; no originals or proxies.
- `Full Cloud Backup`: upload originals, proxies, MHL, reports, and thumbnails.

Object types:

- `Original`: source media.
- `Proxy`: proxy media.
- `MHL`: ASC MHL manifests and chain files.
- `RushesLog`: rushes log entries.
- `Thumbnail`: thumbnails.
- `Report`: HTML/TXT/JSON reports.

Interaction requirements:

- Proxy-only jobs can be created after offload completion or queued as soon as proxy generation completes.
- Proxy-only upload must not block local offload and does not require original media cloud backup to finish.
- Each uploaded proxy still needs an integrity state; the UI should not only say "uploaded".
- The UI must clearly separate review delivery from full cloud backup so users do not mistake proxy delivery for archival.

## P0: Cloud Integrity Verification

Goal: use provider-specific verification where possible and present verification strength honestly in the UI.

Common strategy:

- Create a local manifest for every uploaded object with path, size, mtime, SHA-256, and MD5.
- Before upload, record the source hash; after upload, record remote size, etag, provider hash, part hash, or readback hash.
- If the provider cannot return a trustworthy hash, the object must not be marked as strongly hash-verified.

Google Drive:

- Use resumable upload.
- Fetch file metadata after upload completion.
- Compare `md5Checksum` and size when available; transcoded video or Google-native files are excluded from strong hash decisions.

Baidu Netdisk:

- Use the official multipart flow: precreate, part upload, create.
- Record local block MD5s and whole-file MD5.
- After create, compare remote size, fs_id, md5, or the equivalent fields exposed by the provider.

WebDAV / AList:

- Treat the baseline capability set as write, read, list, and stat.
- At minimum, compare remote size after upload.
- If readback verification is enabled, stream the remote object back and re-hash it; otherwise mark it as size-only verified.

## P1: Provider Support

- Google Drive: OAuth, resumable upload, metadata hash verification, shared link creation.
- Baidu Netdisk: official API provider, multipart upload, rapid-upload detection, token refresh.
- S3: multipart upload, etag interpretation, optional checksum headers.
- WebDAV/AList: keep as experimental and label verification limitations in the UI.

## P1: Queue And Notification UX

- Upload queue filtering by shooting day, project, destination, and object type.
- Optional email, Feishu, or Slack notification after upload completion.
- Delivery summary containing only proxy links and report links.
- Move credentials into system Keychain so config files do not store plaintext tokens.

## Acceptance Tests

- Slow provider simulation: throttling, jitter, disconnects, timeouts, 429, and 5xx retries.
- Source card release after local offload, while cloud upload continues from the destination volume.
- New local offload during cloud upload can scan, copy, verify, and generate MHL normally.
- New cloud upload during an existing upload follows queue and concurrency settings.
- Proxy-only jobs upload only proxy and report-class objects, never originals.
- Verification failure triggers retry; after retry exhaustion, the job fails with diagnostics retained.
- App restart resumes incomplete uploads without re-uploading objects already uploaded and verified.
