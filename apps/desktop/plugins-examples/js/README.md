# example-js · JavaScript plugin template

The simplest possible Bilusic JS plugin — useful as a starting point for
authors who want to write a metadata source in JavaScript instead of
writing a Rust guest for the WASM runtime or hand-rolling a declarative
manifest. It demonstrates the **full `op` dispatch convention** plus a real
HTTP call via `bilusic.httpGet`.

## What it shows

- The `main(inputJson) → string` contract (parse → JSON.stringify)
- **Phase IV `op` dispatch** — `search` / `get_track` / `get_lyrics` /
  `home` / `toplist`, with a legacy `{query}` / `{id}` fallback
- How `bilusic.log(level, msg)` and `bilusic.httpGet(url, optsJson)` are
  wired in by the host prelude
- A small in-memory mock dataset that demonstrates every response shape
  (swap the mock lookup for a real `bilusic.httpGet` call to go live)
- The recommended IIFE-wrapped + `globalThis.main = main` assignment
  pattern so the plugin behaves identically when evaluated as a script
  (current rquickjs 0.12 path) or as a real ES module in a future release

## The `op` convention

Every call from the host is a single JSON string. The host stamps an `op`
field so one `main` can serve all trait methods:

| `op`        | Input                                   | Output                                    |
|-------------|-----------------------------------------|-------------------------------------------|
| `search`    | `{"op":"search","query","page"}`        | `{"tracks":[<MetadataTrack>]}`            |
| `get_track` | `{"op":"get_track","id"}`               | `{"track":<MetadataTrack>}`               |
| `get_lyrics`| `{"op":"get_lyrics","track"}`           | `{"lyrics":{"lines":[{"text","ms"}]}}`    |
| `home`      | `{"op":"home"}`                         | `{"home":{"playlists","new_songs",...}}`  |
| `toplist`   | `{"op":"toplist","topid"}`              | `{"songs":[{"track","reason"}]}`          |

Legacy plugins that only sent `{"query":...}` / `{"id":...}` still resolve
through the fallback in `main()` — but new plugins should always use `op`.

## HTTP binding

`bilusic.httpGet(url, optsJson)` returns `{ ok, status, headers, body }`
synchronously (the host drives the async reqwest call internally). Headers
and options are passed as a JSON string:

```js
const res = bilusic.httpGet(
    'https://api.example.com/search?q=' + encodeURIComponent(query),
    JSON.stringify({ headers: { 'User-Agent': 'bilusic-js' } })
);
if (res.ok) {
    const data = JSON.parse(res.body);
    // map data -> MetadataTrack
}
```

`bilusic.httpPost(url, body, optsJson)` and
`bilusic.b64decode_alphabet(str, alphabet, offset)` are also available for
POST endpoints and custom base64 alphabets (e.g. Kuwo's rotated alphabet).

## Wiring

`plugin.json` declares `"runtime": "js"` and `"entry": "plugin.js"`. The
host reads the file, builds a fresh QuickJS context per call, evaluates the
prelude (which sets up `bilusic.*`) followed by `plugin.js`, then calls
`main(...)` for every request.

## Build prerequisite

The `js` feature is on by default in this project, but if you build a
lean binary you must enable it:

```bash
cargo build --features js
# or
tauri dev --features js
```

Without `--features js` the manifest falls through to the
"需要 --features js" stub branch and `example-js` shows up as
「占位」in Settings → Plugins.

## See also

- `itunes-declarative/` — declarative (zero-code) equivalent
- `example-wasm/` — WASM (Rust) equivalent
- `plugins/kuwo/` — a production JS plugin (full 5-endpoint Kuwo source)
- `plugins/README.md` — full plugin authoring guide
