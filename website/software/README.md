# Website Software Downloads

This directory is intentional and must not be treated as cleanup junk.

DIT Pro is deployed through Vercel, which is reachable from mainland China. GitHub Releases can be unreliable or inaccessible for users in mainland China, so the website keeps installer backups in this directory and uses them as same-origin fallback downloads.

Download routing:

- If GitHub is reachable, the website can prefer GitHub Release assets.
- If GitHub is not reachable, the website falls back to `./software/*` files served by the Vercel deployment.
- `latest.json` and `latest-beta.json` describe both the GitHub source assets and the Vercel backup URLs.

Cleanup rule:

- Do not delete `*.dmg`, `*.exe`, or `latest*.json` from this directory during local repository cleanup.
- Only replace or remove these files as part of a deliberate release/distribution update.
