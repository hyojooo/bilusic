// Bilusic JavaScript plugin example.
//
// Contract (P5-III, js feature, Phase IV):
//   `main(inputJson: string) → string` returning JSON.
//   The plugin is responsible for JSON.parse / JSON.stringify at both ends;
//   the host always passes a JSON string and expects a JSON string back.
//   This keeps the Rust ↔ JS boundary a single string type and sidesteps
//   rquickjs 0.12's value-conversion surface area.
//
// Dispatch uses an `op` field (Phase IV). Legacy plugins that only sent
// { query } / { id } still work via the fallback in main() below.
//
// Input shape per op:
//   search      → {"op":"search","query":"...","page":1}
//   get_track   → {"op":"get_track","id":"..."}
//   get_lyrics  → {"op":"get_lyrics","track":<MetadataTrack>}
//   home        → {"op":"home"}
//   toplist     → {"op":"toplist","topid":<number>}
//
// Return shape (after JSON.stringify):
//   search      → {"tracks":[<MetadataTrack>, ...]}   (bare array also accepted)
//   get_track   → {"track":<MetadataTrack>}           (bare object also accepted)
//   get_lyrics  → {"lyrics":{"lines":[{"text","ms"}]}}  ({"lines"} / bare array also accepted)
//   home        → {"home":{"playlists":[<FeedItem>],"new_songs":[<FeedSong>],
//                           "new_albums":[<FeedItem>],"artists":[<FeedItem>]}}
//   toplist     → {"songs":[{"track":<MetadataTrack>,"reason":<string>}]}
//
// bilusic global (injected by the host prelude, see js_runtime.rs):
//   bilusic.source / .name / .version / .capabilities / .log(level, msg)
//   bilusic.httpGet(url, optsJson)  → {ok,status,headers,body}
//   bilusic.httpPost(url, body, optsJson) → {ok,status,headers,body}
//   bilusic.b64decode_alphabet(str, alphabet, offset) → decoded string
//
// This template demonstrates the full `op` dispatch plus a real HTTP call
// via bilusic.httpGet, using a small mock dataset as the offline fallback.

(function () {
    const SRC = 'example-js';

    const MOCK_TRACKS = [
        {
            source_id: 'js-mock-1',
            title: 'JavaScript Test Track',
            artist: 'Bilusic JS Plugin',
            album: 'P5-III Examples',
            duration_ms: 180000,
            cover: '',
            lyrics_id: null,
            engine_hint: 'bilibili',
        },
        {
            source_id: 'js-mock-2',
            title: 'Mock Sunset',
            artist: 'JS Composer',
            album: 'Mojave Skies',
            duration_ms: 240000,
            cover: '',
            lyrics_id: null,
            engine_hint: 'bilibili',
        },
    ];

    // Thin wrapper over the host HTTP binding.
    function httpGet(url, extraHeaders) {
        const res = bilusic.httpGet(url, JSON.stringify(extraHeaders || {}));
        if (!res.ok) {
            throw new Error('HTTP ' + res.status + ' for ' + url);
        }
        return res.body;
    }

    function searchOp(query, page) {
        // Real plugins call httpGet(<search url>) here and map the response
        // to MetadataTrack objects. We fall back to the mock dataset so the
        // template stays offline-safe.
        const q = String(query || '').toLowerCase();
        const matches = MOCK_TRACKS.filter(t =>
            t.title.toLowerCase().includes(q) ||
            t.artist.toLowerCase().includes(q)
        );
        return { tracks: matches };
    }

    function getTrackOp(id) {
        const t = MOCK_TRACKS.find(x => x.source_id === id);
        if (!t) {
            // Mirror declarative's permissive "empty record" behaviour.
            return { track: {
                source_id: id,
                title: 'Unknown',
                artist: '',
                album: '',
                duration_ms: 0,
                cover: '',
                lyrics_id: null,
            } };
        }
        return { track: t };
    }

    function getLyricsOp(track) {
        // Real plugins call httpGet(<lyric url>) here and parse lrclist[].
        return { lyrics: { lines: [] } };
    }

    function homeOp() {
        return { home: {
            playlists: [{ id: SRC + ':pl:1', title: 'Example Playlist', subtitle: 'curated', cover: '', source_id: SRC }],
            new_songs: MOCK_TRACKS.map(t => Object.assign({}, t)),
            new_albums: [],
            artists: [{ id: SRC + ':ar:1', title: 'Example Artist', subtitle: '', cover: '', source_id: SRC }],
        } };
    }

    function toplistOp(topid) {
        return { songs: MOCK_TRACKS.map(t => ({ track: t, reason: 'top ' + topid })) };
    }

    function main(inputJson) {
        const input = JSON.parse(inputJson);
        bilusic.log('info', 'example-js called op=' + (input.op || '?'));

        // Phase IV dispatch; legacy plugins that only sent {query}/{id}
        // still resolve correctly through the fallback.
        const op = input.op
            || (input.query !== undefined ? 'search' : null)
            || (input.id !== undefined ? 'get_track' : null);

        switch (op) {
            case 'search':     return JSON.stringify(searchOp(input.query, input.page || 1));
            case 'get_track':  return JSON.stringify(getTrackOp(input.id));
            case 'get_lyrics': return JSON.stringify(getLyricsOp(input.track));
            case 'home':       return JSON.stringify(homeOp());
            case 'toplist':    return JSON.stringify(toplistOp(input.topid));
            default:           return '{}';
        }
    }

    // Expose to the host. rquickjs evaluates the source as a top-level
    // script, so `function main(...)` declared at script-level is auto-
    // bound to `globalThis.main`; we don't need `module.exports`.
    if (typeof globalThis !== 'undefined') {
        globalThis.main = main;
    }
})();
