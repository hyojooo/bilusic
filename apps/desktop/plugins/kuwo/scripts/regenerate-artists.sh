#!/usr/bin/env bash
# Regenerate the baked artist feed for the Kuwo JS plugin.
#
# Why this exists (N11i-4):
#   The Kuwo artistInfo endpoint at
#   /api/www/artist/artistInfo sits behind the same anti-bot wall that
#   ate /api/www/search/... (N11h). It needs:
#     * the official `Secret` header (rotates)
#     * a site-scoped Baidu-Tongji `Hm_Iuvt_...` cookie
#     * Chrome 151 UA
#     * `Referer: http://www.kuwo.cn/rankList`
#   We don't mint any of those from a desktop client; instead we bake
#   a 60-row snapshot at build time and ship the JS plugin with it
#   inline. New users get the snapshot, no HTTP call to artistInfo.
#
# Usage:
#   1. Open https://www.kuwo.cn/rankList in your desktop Chrome, copy
#      the live `Secret` request header value (DevTools Network → the
#      artistInfo XHR → Headers → "Secret"), and paste it below where
#      marked SECRET_VALUE.
#   2. Optionally also copy the `Cookie: Hm_Iuvt_...=...` value from
#      the same request — if Kuwo's rotating the secret more
#      aggressively than usual, the cookie is what makes the wall
#      accept the request.
#   3. Run `bash scripts/regenerate-artists.sh` from this directory.
#      Outputs:
#        - data/artists-raw.json    (full upstream payload, for debugging)
#        - data/artists-baked.json  (stripped 60-row normalised form)
#   4. Diff the new BAKED_ARTISTS_RAW literal in `index.js` and replace
#      the existing one (see the `=== BAKED ARTISTS DATA ===` block at
#      the top of the file for the inline block + provenance notes).
#
# Refresh cadence: quarterly. The list is "top 60 artists" by
# popularity, so stale is mostly harmless — but new chart-toppers won't
# appear until you re-bake.
set -euo pipefail

cd "$(dirname "$0")/.."

SECRET_VALUE="${KUWO_SECRET:-4812c9ca5a122651841dd4a423f1f65e62b6ba8f74aed45047b729ead1b335e2011dce83}"
COOKIE_VALUE="${KUWO_COOKIE:-Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=kBBhK5NzZ8t5hKGsjyKXCTHWN7RsW5Pn}"
UA='Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36'
REQQID="$(uuidgen 2>/dev/null || python3 -c 'import uuid;print(uuid.uuid4())')"
URL='http://www.kuwo.cn/api/www/artist/artistInfo?category=0&pn=1&rn=60&httpsStatus=1&reqId='"$REQQID"'&plat=web_www'

echo "[regenerate-artists] GET $URL"
echo "[regenerate-artists] reqId=$REQQID"
echo "[regenerate-artists] Secret=${SECRET_VALUE:0:8}... (truncated)"

# 1. fetch raw
curl --silent --show-error --insecure --max-time 20 \
  --url "$URL" \
  -H "Accept: application/json, text/plain, */*" \
  -H "Accept-Language: zh-CN,zh;q=0.5" \
  -H "Cache-Control: no-cache" \
  -b "$COOKIE_VALUE" \
  -H "Pragma: no-cache" \
  -H "Proxy-Connection: keep-alive" \
  -H "Referer: http://www.kuwo.cn/rankList" \
  -H "Sec-GPC: 1" \
  -H "Secret: $SECRET_VALUE" \
  -H "User-Agent: $UA" \
  -o data/artists-raw.json

# 2. validate + extract
if ! command -v node >/dev/null 2>&1; then
  echo "[regenerate-artists] FATAL: node is required to post-process (extract fields)." >&2
  exit 1
fi

node - <<'EOF'
const fs = require("fs");
// `cd "$(dirname "$0")/.."` already moved us to plugins/kuwo, so
// process.cwd() is the right base.
const base = process.cwd();
const rawPath = base + "/data/artists-raw.json";
const outPath = base + "/data/artists-baked.json";

let raw;
try {
  raw = JSON.parse(fs.readFileSync(rawPath, "utf8"));
} catch (e) {
  console.error("[regenerate-artists] FATAL: failed to parse " + rawPath + ": " + e.message);
  process.exit(2);
}
if (!raw || raw.code !== 200 || !raw.data || !Array.isArray(raw.data.artistList)) {
  console.error("[regenerate-artists] FATAL: unexpected response shape — code=" + (raw && raw.code) + ", topKeys=" + Object.keys(raw || {}).join(","));
  process.exit(3);
}

const list = raw.data.artistList.map((a) => ({
  id: a.id,
  name: a.name,
  aartist: a.aartist,
  pic: a.pic,
  pic300: a.pic300,
  artistFans: a.artistFans,
  albumNum: a.albumNum,
  musicNum: a.musicNum,
}));

fs.writeFileSync(outPath, JSON.stringify(list, null, 2));
console.log("[regenerate-artists] wrote " + list.length + " artists to " + outPath);
console.log("[regenerate-artists] sample names: " + list.slice(0, 5).map((a) => a.name).join(", "));
EOF

echo "[regenerate-artists] DONE."
echo "[regenerate-artists] next step: open index.js, find the '=== BAKED ARTISTS DATA ==='"
echo "[regenerate-artists] block, and replace BAKED_ARTISTS_RAW with the contents of"
echo "[regenerate-artists] data/artists-baked.json (compact one-line form)."
