// 酷我音乐插件（JS，phase IV）
//
// Endpoints implemented:
//   op="search"       →  www.kuwo.cn/search/searchMusicBykeyWord  (N11i-3, current)
//   op="get_track"    →  www.kuwo.cn/api/www/music/musicInfo  (N11h, MIGRATE-TO-/search/-pending)
//   op="get_lyrics"   →  www.kuwo.cn/api/v1/www/lyric/getLyricByLine  (N11h, MIGRATE-TO-/search/-pending)
//   op="home"         →  新歌首发 (musicList?bangId=17) + 6 特色榜单 (musicList?bangId=26/16/284/64/291/331) + 热门歌手 (artistInfo 运行时优先+baked 兜底)
//   op="toplist"      →  songs of a single bang (via search)
//
// Endpoint evolution:
//   - The original mobile endpoint `search.kuwo.cn/KuwoMobileApi/...`
//     is dead (returns empty body — verified 2026-08).
//   - N11h + N11i-2 lived on `www.kuwo.cn/api/www/search/...` with the
//     CSRF + browser-fingerprint double-check, and *every* attempt —
//     crafted csrf token, anonymous bare-headers, full Chrome-headers
//     — kept returning `{"success":false,"message":"The request is illegal!"}`.
//   - N11i-3 (this version) switches `search` to
//     `www.kuwo.cn/search/searchMusicBykeyWord?vipver=1&client=kt&...`.
//     This is the endpoint the www.kuwo.cn search page itself calls
//     (captured via Postman from a real browser session). It is more
//     lenient on the fingerprint check, accepts the same anonymous
//     full-browser-header set we ship, and returns its song array
//     under `data.abslist` (with `songs`/`list`/`abslist` variants
//     all auto-detected by `pickSongList`). Response shapes vary, so
//     searchOp dumps the full body to the Console on every call —
//     paste it back here and we adapt.
//
// Header model (N11i-3, refined from N11i-2): every request goes out
// with a Postman-shaped browser header set — User-Agent / Accept /
// Accept-Language / X-Requested-With / Origin / Referer / Connection.
// We explicitly drop Sec-Fetch-{Site,Mode,Dest} (Postman doesn't ship
// them; including them while keeping everything else Postman-shaped
// is worse than omitting them, per N11i-3 audit). No csrf token, no
// kw_token cookie. Host field is omitted — reqwest fills it in from
// the URL, avoiding any Host/Origin/URL-host drift.
//
// All HTTP goes through bilusic.httpGet (sync, returns {ok,status,headers,
// body}). A 30-second in-memory cache keeps `home` from hammering the API.
//
// First-party host functions available:
//   bilusic.httpGet(url, optionsJson)  → { ok, status, headers, body }
//   bilusic.httpPost(url, body, optionsJson)
//   bilusic.b64decode_alphabet(s, alphabet, offset)
//   bilusic.log(level, msg)
//
// optionsJson shape:
//   { "headers": { "User-Agent": "...", "Referer": "..." } }
//
// ====================================================================
// === BAKED ARTISTS DATA (do not edit by hand) ========================
// ===
// === Source: GET http://www.kuwo.cn/api/www/artist/artistInfo
// ===         ?category=0&pn=1&rn=60&httpsStatus=1
// ===         &reqId=b64a84f0-9d44-11f1-aa40-737b12c022a3
// ===         &plat=web_www
// === Captured: 2026-08-21 (curl from sandbox with the same Secret+Cookie
// ===          + Chrome 151 UA Kuwo accepts; see scripts/regenerate-artists.sh)
// === Count: 60 artists (rn=60)
// ===
// === N11i-4 policy: new users of the Kuwo metadata source get this
// === 60-row list served directly to the home page; we DO NOT make a
// === HTTP call to /api/www/artist/artistInfo at runtime. The raw
// === endpoint sits behind the same anti-bot wall that ate the search
// === endpoint (N11h) and is even less reliable than /search/ — the
// === official `Secret` header rotates, and the cookie is site-scoped
// === Baidu-Tongji-style telemetry, not something we should mint from
// === a desktop client. Bake once at build time; refresh quarterly.
// ===
// === To regenerate:
// ===   bash scripts/regenerate-artists.sh    (re-bakes data/artists-raw.json + data/artists-baked.json)
// ===   then copy the new compact JSON into the BAKED_ARTISTS_RAW below.
// ====================================================================
const BAKED_ARTISTS_RAW = [
  {
    id: 336,
    name: '周杰伦',
    aartist: 'Jay Chou',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s56/58/291211030.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s56/58/291211030.jpg',
    artistFans: 1664361,
    albumNum: 45,
    musicNum: 1697,
  },
  {
    id: 909,
    name: '莫文蔚',
    aartist: 'Karen Mok',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s57/57/2536086177.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s57/57/2536086177.jpg',
    artistFans: 115166,
    albumNum: 83,
    musicNum: 763,
  },
  {
    id: 1062,
    name: '林俊杰',
    aartist: 'JJ Lin',
    pic: 'https://img3.kuwo.cn/star/starheads/300/40/64/2063938955.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/40/64/2063938955.jpg',
    artistFans: 818708,
    albumNum: 81,
    musicNum: 1227,
  },
  {
    id: 47,
    name: '陈奕迅',
    aartist: 'Eason Chan',
    pic: 'https://img4.kuwo.cn/star/starheads/300/37/26/3816222178.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/37/26/3816222178.jpg',
    artistFans: 592556,
    albumNum: 152,
    musicNum: 1838,
  },
  {
    id: 5371,
    name: 'G.E.M. 邓紫棋',
    aartist: '',
    pic: 'https://img4.kuwo.cn/star/starheads/300/40/9/1980630626.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/40/9/1980630626.jpg',
    artistFans: 957941,
    albumNum: 72,
    musicNum: 931,
  },
  {
    id: 1250,
    name: 'BEYOND',
    aartist: '',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s5/17/3477575663.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s5/17/3477575663.jpg',
    artistFans: 300594,
    albumNum: 173,
    musicNum: 1502,
  },
  {
    id: 1416,
    name: '王力宏',
    aartist: 'Leehom Wang',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s63/78/3965267796.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s63/78/3965267796.jpg',
    artistFans: 204924,
    albumNum: 88,
    musicNum: 933,
  },
  {
    id: 2,
    name: '周传雄',
    aartist: 'Steve Chou',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s84/13/1119809443.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s84/13/1119809443.jpg',
    artistFans: 147165,
    albumNum: 45,
    musicNum: 637,
  },
  {
    id: 896,
    name: '张学友',
    aartist: 'Jacky Cheung',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s30/24/2781671968.png',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s30/24/2781671968.png',
    artistFans: 433277,
    albumNum: 144,
    musicNum: 1938,
  },
  {
    id: 286,
    name: '刘德华',
    aartist: 'Andy Lau',
    pic: 'https://img3.kuwo.cn/star/starheads/300/65/42/2631374422.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/65/42/2631374422.jpg',
    artistFans: 375607,
    albumNum: 176,
    musicNum: 2268,
  },
  {
    id: 1892,
    name: '王杰',
    aartist: 'Dave Wang',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s79/6/3934255369.png',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s79/6/3934255369.png',
    artistFans: 123396,
    albumNum: 94,
    musicNum: 915,
  },
  {
    id: 973,
    name: '任贤齐',
    aartist: 'Richie Jen',
    pic: 'https://img2.kuwo.cn/star/starheads/300/55/19/2006209414.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/55/19/2006209414.jpg',
    artistFans: 139881,
    albumNum: 64,
    musicNum: 992,
  },
  {
    id: 1493,
    name: '张信哲',
    aartist: 'Jeff Chang Shin-Che',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s74/42/874158805.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s74/42/874158805.jpg',
    artistFans: 178564,
    albumNum: 84,
    musicNum: 907,
  },
  {
    id: 744,
    name: '孙燕姿',
    aartist: 'Stefanie Sun',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s28/47/3391812016.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s28/47/3391812016.jpg',
    artistFans: 117118,
    albumNum: 56,
    musicNum: 746,
  },
  {
    id: 810,
    name: '谭咏麟',
    aartist: 'Alan Tam',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s44/93/3455715154.png',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s44/93/3455715154.png',
    artistFans: 118303,
    albumNum: 204,
    musicNum: 2448,
  },
  {
    id: 317,
    name: '梁静茹',
    aartist: 'Fish Leong',
    pic: 'https://img2.kuwo.cn/star/starheads/300/0/30/4181357387.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/0/30/4181357387.jpg',
    artistFans: 128915,
    albumNum: 39,
    musicNum: 667,
  },
  {
    id: 492,
    name: '张韶涵',
    aartist: 'Angela Zhang',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s58/68/3701106526.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s58/68/3701106526.jpg',
    artistFans: 354741,
    albumNum: 55,
    musicNum: 719,
  },
  {
    id: 221102,
    name: '赵乃吉',
    aartist: '',
    pic: 'https://img3.kuwo.cn/star/starheads/300/96/70/1767459137.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/96/70/1767459137.jpg',
    artistFans: 11158,
    albumNum: 336,
    musicNum: 1328,
  },
  {
    id: 1359,
    name: '陈小春',
    aartist: 'Jordan Chan',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s50/7/912851190.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s50/7/912851190.jpg',
    artistFans: 55784,
    albumNum: 58,
    musicNum: 650,
  },
  {
    id: 332,
    name: '周华健',
    aartist: 'Emil Chau',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s37/35/355532631.png',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s37/35/355532631.png',
    artistFans: 145063,
    albumNum: 78,
    musicNum: 1051,
  },
  {
    id: 233,
    name: '陈慧娴',
    aartist: 'Priscilla Chan',
    pic: 'https://img2.kuwo.cn/star/starheads/300/22/17/201440287.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/22/17/201440287.jpg',
    artistFans: 100971,
    albumNum: 88,
    musicNum: 813,
  },
  {
    id: 1328,
    name: '邓丽君',
    aartist: 'Teresa Teng',
    pic: 'https://img1.kuwo.cn/star/starheads/300/16/69/2617556901.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/16/69/2617556901.jpg',
    artistFans: 457601,
    albumNum: 523,
    musicNum: 2902,
  },
  {
    id: 223,
    name: '杨丞琳',
    aartist: 'Rainie Yang',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s28/93/3469735896.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s28/93/3469735896.jpg',
    artistFans: 35651,
    albumNum: 47,
    musicNum: 667,
  },
  {
    id: 210,
    name: '伍佰 & China Blue',
    aartist: '',
    pic: 'https://img3.kuwo.cn/star/starheads/300/71/3/2082158453.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/71/3/2082158453.jpg',
    artistFans: 71107,
    albumNum: 44,
    musicNum: 639,
  },
  {
    id: 1209,
    name: '张宇',
    aartist: 'Phil Chang',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s93/89/251061154.png',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s93/89/251061154.png',
    artistFans: 129806,
    albumNum: 25,
    musicNum: 308,
  },
  {
    id: 522,
    name: '蔡健雅',
    aartist: 'Tanya Chua',
    pic: 'https://img3.kuwo.cn/star/starheads/300/97/0/1283488305.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/97/0/1283488305.jpg',
    artistFans: 84173,
    albumNum: 44,
    musicNum: 515,
  },
  {
    id: 624,
    name: 'S.H.E',
    aartist: '',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s47/40/1518389109.png',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s47/40/1518389109.png',
    artistFans: 104680,
    albumNum: 33,
    musicNum: 650,
  },
  {
    id: 833,
    name: '张国荣',
    aartist: 'Leslie Cheung',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s44/6/2602790997.png',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s44/6/2602790997.png',
    artistFans: 162626,
    albumNum: 145,
    musicNum: 1197,
  },
  {
    id: 194445,
    name: '刘珂矣',
    aartist: '',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s12/68/921143849.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s12/68/921143849.jpg',
    artistFans: 113609,
    albumNum: 35,
    musicNum: 217,
  },
  {
    id: 978,
    name: '蔡依林',
    aartist: 'Jolin Cai',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s67/8/3629759113.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s67/8/3629759113.jpg',
    artistFans: 131734,
    albumNum: 73,
    musicNum: 1110,
  },
  {
    id: 4831713,
    name: '老板',
    aartist: '',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s70/86/119174062.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s70/86/119174062.jpg',
    artistFans: 885,
    albumNum: 305,
    musicNum: 898,
  },
  {
    id: 451,
    name: '陶喆',
    aartist: 'David Tao',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s55/31/1316361792.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s55/31/1316361792.jpg',
    artistFans: 45783,
    albumNum: 33,
    musicNum: 502,
  },
  {
    id: 490,
    name: '李克勤',
    aartist: 'Hacken Lee',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s3s19/47/3346755739.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s3s19/47/3346755739.jpg',
    artistFans: 89526,
    albumNum: 131,
    musicNum: 1593,
  },
  {
    id: 1306,
    name: '王心凌',
    aartist: 'Cyndi Wang',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s77/97/1548119791.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s77/97/1548119791.jpg',
    artistFans: 93260,
    albumNum: 43,
    musicNum: 579,
  },
  {
    id: 1431,
    name: '刘若英',
    aartist: 'Ren’e Liu',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s5/79/1902236917.png',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s5/79/1902236917.png',
    artistFans: 139003,
    albumNum: 48,
    musicNum: 586,
  },
  {
    id: 354,
    name: '五月天',
    aartist: 'Mayday',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s1/32/167770198.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s1/32/167770198.jpg',
    artistFans: 138129,
    albumNum: 75,
    musicNum: 1055,
  },
  {
    id: 1014,
    name: '林忆莲',
    aartist: 'Sandy Lam',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s28/70/1001193820.png',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s28/70/1001193820.png',
    artistFans: 98967,
    albumNum: 122,
    musicNum: 1142,
  },
  {
    id: 12635726,
    name: 'ProdbyMend',
    aartist: '',
    pic: 'https://img3.kuwo.cn/star/starheads/300/20/57/3711657954.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/20/57/3711657954.jpg',
    artistFans: 7,
    albumNum: 918,
    musicNum: 3548,
  },
  {
    id: 5265,
    name: '杨宗纬',
    aartist: 'Aska Yang',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s56/96/3453738367.png',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s56/96/3453738367.png',
    artistFans: 110819,
    albumNum: 44,
    musicNum: 665,
  },
  {
    id: 226,
    name: '萧亚轩',
    aartist: 'Elva Hsiao',
    pic: 'https://img3.kuwo.cn/star/starheads/300/37/30/674377541.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/37/30/674377541.jpg',
    artistFans: 67414,
    albumNum: 34,
    musicNum: 541,
  },
  {
    id: 4968,
    name: 'Taylor Swift',
    aartist: '泰勒·斯威夫特',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s85/29/2408248969.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s85/29/2408248969.jpg',
    artistFans: 364700,
    albumNum: 204,
    musicNum: 1529,
  },
  {
    id: 922,
    name: '阿杜',
    aartist: 'A do',
    pic: 'https://img3.kuwo.cn/star/starheads/300/78/15/2244879021.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/78/15/2244879021.jpg',
    artistFans: 56984,
    albumNum: 29,
    musicNum: 302,
  },
  {
    id: 1045,
    name: '陈百强',
    aartist: 'Danny Chan',
    pic: 'https://img2.kuwo.cn/star/starheads/300/s4s1/5/1960802277.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/s4s1/5/1960802277.jpg',
    artistFans: 59881,
    albumNum: 68,
    musicNum: 523,
  },
  {
    id: 1072,
    name: '谢霆锋',
    aartist: 'Nicholas Tse',
    pic: 'https://img1.kuwo.cn/star/starheads/300/60/45/3679782984.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/60/45/3679782984.jpg',
    artistFans: 42335,
    albumNum: 66,
    musicNum: 1142,
  },
  {
    id: 948,
    name: '李翊君',
    aartist: 'E-Jun Lee',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s44/50/2124727904.png',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s44/50/2124727904.png',
    artistFans: 43540,
    albumNum: 71,
    musicNum: 681,
  },
  {
    id: 1094,
    name: '李宗盛',
    aartist: 'Jonathan Lee',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s97/63/3770055801.png',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s97/63/3770055801.png',
    artistFans: 141332,
    albumNum: 20,
    musicNum: 381,
  },
  {
    id: 6989323,
    name: 'Grimmmz',
    aartist: '',
    pic: 'https://img4.kuwo.cn/star/starheads/300/19/26/2714914296.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/19/26/2714914296.jpg',
    artistFans: 23,
    albumNum: 9436,
    musicNum: 9990,
  },
  {
    id: 125824,
    name: '金润吉',
    aartist: 'A RUN',
    pic: 'https://img1.kuwo.cn/star/starheads/300/97/95/3186465824.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/97/95/3186465824.jpg',
    artistFans: 18145,
    albumNum: 122,
    musicNum: 487,
  },
  {
    id: 1367,
    name: '杨千嬅',
    aartist: 'Miriam Yeung',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s27/71/2446925024.png',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s27/71/2446925024.png',
    artistFans: 43877,
    albumNum: 114,
    musicNum: 1136,
  },
  {
    id: 1889,
    name: '韩宝仪',
    aartist: 'HanBaoyi',
    pic: 'https://img2.kuwo.cn/star/starheads/300/57/0/1536451762.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/57/0/1536451762.jpg',
    artistFans: 142078,
    albumNum: 248,
    musicNum: 1654,
  },
  {
    id: 36,
    name: '叶蒨文',
    aartist: 'Sally Yeh',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s3/54/2875723013.png',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s3/54/2875723013.png',
    artistFans: 68975,
    albumNum: 68,
    musicNum: 904,
  },
  {
    id: 1001,
    name: '张惠妹',
    aartist: 'A-Mei',
    pic: 'https://img2.kuwo.cn/star/starheads/300/19/60/3384158984.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/19/60/3384158984.jpg',
    artistFans: 93602,
    albumNum: 61,
    musicNum: 1135,
  },
  {
    id: 314,
    name: '潘玮柏',
    aartist: 'Wilber Pan',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s57/75/3416007315.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s57/75/3416007315.jpg',
    artistFans: 40609,
    albumNum: 38,
    musicNum: 551,
  },
  {
    id: 678,
    name: '草蜢',
    aartist: '草蜢乐队',
    pic: 'https://img3.kuwo.cn/star/starheads/300/s4s49/70/3470179109.jpg',
    pic300: 'https://img3.kuwo.cn/star/starheads/300/s4s49/70/3470179109.jpg',
    artistFans: 20534,
    albumNum: 77,
    musicNum: 870,
  },
  {
    id: 983,
    name: '方大同',
    aartist: 'Khalil Fong',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s24/98/3925627773.png',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s24/98/3925627773.png',
    artistFans: 19955,
    albumNum: 55,
    musicNum: 549,
  },
  {
    id: 226572,
    name: 'Alan Walker',
    aartist: '艾伦·沃克',
    pic: 'https://img1.kuwo.cn/star/starheads/300/14/7/272889291.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/14/7/272889291.jpg',
    artistFans: 641631,
    albumNum: 199,
    musicNum: 745,
  },
  {
    id: 69677,
    name: '唐伯虎Annie',
    aartist: '',
    pic: 'https://img2.kuwo.cn/star/starheads/300/7/78/603484591.jpg',
    pic300: 'https://img2.kuwo.cn/star/starheads/300/7/78/603484591.jpg',
    artistFans: 23512,
    albumNum: 121,
    musicNum: 523,
  },
  {
    id: 788,
    name: '郭静',
    aartist: 'Claire',
    pic: 'https://img1.kuwo.cn/star/starheads/300/s4s3/67/1698836953.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/s4s3/67/1698836953.jpg',
    artistFans: 24156,
    albumNum: 63,
    musicNum: 339,
  },
  {
    id: 871,
    name: '陈慧琳',
    aartist: 'Kelly Chen',
    pic: 'https://img4.kuwo.cn/star/starheads/300/s4s53/57/1903031562.jpg',
    pic300: 'https://img4.kuwo.cn/star/starheads/300/s4s53/57/1903031562.jpg',
    artistFans: 38908,
    albumNum: 113,
    musicNum: 1287,
  },
  {
    id: 68432,
    name: 'F.I.R.飞儿乐团',
    aartist: 'F.I.R.',
    pic: 'https://img1.kuwo.cn/star/starheads/300/92/72/2599301580.jpg',
    pic300: 'https://img1.kuwo.cn/star/starheads/300/92/72/2599301580.jpg',
    artistFans: 23480,
    albumNum: 33,
    musicNum: 387,
  },
];
// Baked artists, normalised into the home-feed shape. We strip the
// numeric/field-heavy original into {id, title, subtitle, cover, kind,
// source} so the home renderer can consume the same data shape as a
// live-fetched artist list would. (subtitle falls back to aartist
// when the Chinese name already contains a stage name like 'G.E.M. 邓紫棋'.)
const BAKED_ARTISTS = (function () {
  var out = [];
  for (var i = 0; i < BAKED_ARTISTS_RAW.length; i++) {
    var a = BAKED_ARTISTS_RAW[i];
    out.push({
      id: a.id,
      title: a.name,
      subtitle: a.aartist || null,
      cover: a.pic300 || a.pic || '',
      kind: 'artist',
      source: 'kuwo',
    });
  }
  return out;
})();

const REFERER_BASE = 'https://www.kuwo.cn/search/list?key=';
const DESKTOP_UA =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';

const ARTIST_UA =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36';

const ARTIST_SECRET =
  '4812c9ca5a122651841dd4a423f1f65e62b6ba8f74aed45047b729ead1b335e2011dce83';
const ARTIST_COOKIE =
  'Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=' +
  'kBBhK5NzZ8t5hKGsjyKXCTHWN7RsW5Pn';
const ARTIST_REFERER = 'http://www.kuwo.cn/rankList';
const ARTIST_INFO_URL =
  'https://www.kuwo.cn/api/www/artist/artistInfo' +
  '?category=0&pn=1&rn=60&httpsStatus=1&plat=web_www&reqId=';

// 榜单详情接口（musicList）专用三件套：Secret + Cookie + Chrome151 UA。
// 与 artistInfo 接口同源（都挂在 /api/www 下），但签名不同，单独一组常量避免互相污染。
// Secret 失效则 bangMusicListOp 返回 null -> 该区块跳过。
const BANG_UA =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ' +
  '(KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36';
const BANG_SECRET =
  '6228d1aa3220401ca1689f9e37ad6f487638543661b78af08bbd081e06658b3b00413cd5';
const BANG_COOKIE =
  'Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=' +
  'AnsNFY4MdbST4rwZXkrmm7jSFCXBaAch';
const BANG_REFERER = 'http://www.kuwo.cn/rankList';
const BANG_MUSIC_LIST_URL =
  'https://www.kuwo.cn/api/www/bang/bang/musicList' +
  '?httpsStatus=1&plat=web_www&from=';

// ====================================================================
// === BAKED BANG DATA (auto-generated; do not edit by hand) =========
// === Snapshot of all 6 bang lists from 2026-08-22 (rn=20 each).
// === Used ONLY as fallback when /api/www/bang/bang/musicList fails
// === (Secret rotated, IP rate-limited, code=-1, illegal, etc.).
// === Runtime fetch wins whenever it succeeds; baked is the safety net.
// === Regenerate via: scripts/regenerate-bang.sh (future).
// ====================================================================
// 经典怀旧榜（bangId=26）— 抓取于 2026-08-22 00:39，rn=20 命中 20 首
const BAKED_BANG_CLASSIC_RAW =
  '[{"id":"MUSIC_1044318","title":"黄昏","subtitle":"Transfer","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s49/14/2320162909.jpg","kind":"song","source":"kuwo","artist":"周传雄","duration_ms":344000,"engine_hint":"bilibili"},{"id":"MUSIC_320462","title":"青花","subtitle":"蓝色土耳其","cover":"https://img4.kuwo.cn/star/albumcover/500/64/80/143067840.jpg","kind":"song","source":"kuwo","artist":"周传雄","duration_ms":297000,"engine_hint":"bilibili"},{"id":"MUSIC_1035026","title":"雨爱","subtitle":"雨爱","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s6/93/1823888016.jpg","kind":"song","source":"kuwo","artist":"杨丞琳","duration_ms":260000,"engine_hint":"bilibili"},{"id":"MUSIC_501646","title":"奢香夫人","subtitle":"最炫民族风","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s4/17/3425185860.jpg","kind":"song","source":"kuwo","artist":"凤凰传奇","duration_ms":259000,"engine_hint":"bilibili"},{"id":"MUSIC_133360","title":"樱花草","subtitle":"花言乔语 (精装版)","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s24/5/661213926.jpg","kind":"song","source":"kuwo","artist":"Sweety","duration_ms":283000,"engine_hint":"bilibili"},{"id":"MUSIC_93151","title":"美人鱼","subtitle":"第二天堂","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s60/86/1922632933.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":254000,"engine_hint":"bilibili"},{"id":"MUSIC_450444","title":"红色高跟鞋","subtitle":"若你碰到他","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s15/81/2985702596.jpg","kind":"song","source":"kuwo","artist":"蔡健雅","duration_ms":206000,"engine_hint":"bilibili"},{"id":"MUSIC_5886682","title":"海阔天空","subtitle":"乐与怒","cover":"https://img1.kuwo.cn/star/albumcover/500/28/42/1776247091.jpg","kind":"song","source":"kuwo","artist":"BEYOND","duration_ms":324000,"engine_hint":"bilibili"},{"id":"MUSIC_637506854","title":"旧记忆","subtitle":"旧记忆","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s83/48/86191593.jpg","kind":"song","source":"kuwo","artist":"黄子韬&刘宇宁","duration_ms":216000,"engine_hint":"bilibili"},{"id":"MUSIC_93157","title":"江南","subtitle":"第二天堂","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s60/86/1922632933.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":267000,"engine_hint":"bilibili"},{"id":"MUSIC_203164","title":"西海情歌","subtitle":"刀郎Ⅲ","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s96/93/2630192267.jpg","kind":"song","source":"kuwo","artist":"刀郎","duration_ms":343000,"engine_hint":"bilibili"},{"id":"MUSIC_1697454","title":"盛夏的果实","subtitle":"NO.1新曲精选全记录20首","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s49/58/3052577581.jpg","kind":"song","source":"kuwo","artist":"莫文蔚","duration_ms":251000,"engine_hint":"bilibili"},{"id":"MUSIC_499778","title":"偏爱","subtitle":"破天荒","cover":"https://img4.kuwo.cn/star/albumcover/500/50/26/402483447.jpg","kind":"song","source":"kuwo","artist":"张芸京","duration_ms":212000,"engine_hint":"bilibili"},{"id":"MUSIC_465829","title":"情歌","subtitle":"靜茹 & 情歌 別再为他流泪","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s34/78/3678542361.jpg","kind":"song","source":"kuwo","artist":"梁静茹","duration_ms":260000,"engine_hint":"bilibili"},{"id":"MUSIC_102428","title":"爱错","subtitle":"心中的日月","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s19/78/3781827790.jpg","kind":"song","source":"kuwo","artist":"王力宏","duration_ms":238000,"engine_hint":"bilibili"},{"id":"MUSIC_80403","title":"十年","subtitle":"黑白灰","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s86/18/1347897214.jpg","kind":"song","source":"kuwo","artist":"陈奕迅","duration_ms":205000,"engine_hint":"bilibili"},{"id":"MUSIC_231323","title":"我怀念的","subtitle":"逆光","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s77/72/859465391.jpg","kind":"song","source":"kuwo","artist":"孙燕姿","duration_ms":289000,"engine_hint":"bilibili"},{"id":"MUSIC_628909","title":"失眠","subtitle":"Ladies Night","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s36/94/1756388171.jpg","kind":"song","source":"kuwo","artist":"Suki刘舒妤","duration_ms":211000,"engine_hint":"bilibili"},{"id":"MUSIC_502980","title":"心墙","subtitle":"在树上唱歌","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s70/15/775573712.jpg","kind":"song","source":"kuwo","artist":"郭静","duration_ms":227000,"engine_hint":"bilibili"},{"id":"MUSIC_228908","title":"晴天","subtitle":"叶惠美","cover":"https://img2.kuwo.cn/star/albumcover/500/s3s94/93/211513640.jpg","kind":"song","source":"kuwo","artist":"周杰伦","duration_ms":269000,"engine_hint":"bilibili"}]';

// 酷我热歌榜（bangId=16）— 抓取于 2026-08-22 00:39，rn=20 命中 20 首
const BAKED_BANG_HOT_RAW =
  '[{"id":"MUSIC_624683929","title":"山风山风等等我","subtitle":"山风山风等等我","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s81/95/2497366108.jpg","kind":"song","source":"kuwo","artist":"万海东","duration_ms":209000,"engine_hint":"bilibili"},{"id":"MUSIC_567247828","title":"你有没有真的爱过我","subtitle":"你有没有真的爱过我","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s54/51/1818397081.jpg","kind":"song","source":"kuwo","artist":"阿图表妹","duration_ms":243000,"engine_hint":"bilibili"},{"id":"MUSIC_570482802","title":"岁月如笔写春秋","subtitle":"岁月如笔写春秋","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s80/50/3012183677.jpg","kind":"song","source":"kuwo","artist":"河南三妹5233","duration_ms":251000,"engine_hint":"bilibili"},{"id":"MUSIC_550221152","title":"人生路漫漫","subtitle":"人生路漫漫","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s93/35/2013522491.jpg","kind":"song","source":"kuwo","artist":"白小白","duration_ms":235000,"engine_hint":"bilibili"},{"id":"MUSIC_595898177","title":"踏马寻花向自由(雷鬼版)","subtitle":"踏马寻花向自由（雷鬼版）","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s98/67/1050454242.jpg","kind":"song","source":"kuwo","artist":"超哥","duration_ms":179000,"engine_hint":"bilibili"},{"id":"MUSIC_1044318","title":"黄昏","subtitle":"Transfer","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s49/14/2320162909.jpg","kind":"song","source":"kuwo","artist":"周传雄","duration_ms":344000,"engine_hint":"bilibili"},{"id":"MUSIC_193290598","title":"如愿","subtitle":"如愿","cover":"https://img3.kuwo.cn/star/albumcover/500/15/57/2715116001.jpg","kind":"song","source":"kuwo","artist":"王菲","duration_ms":265000,"engine_hint":"bilibili"},{"id":"MUSIC_320462","title":"青花","subtitle":"蓝色土耳其","cover":"https://img4.kuwo.cn/star/albumcover/500/64/80/143067840.jpg","kind":"song","source":"kuwo","artist":"周传雄","duration_ms":297000,"engine_hint":"bilibili"},{"id":"MUSIC_22807949","title":"半壶纱","subtitle":"半壶纱","cover":"https://img1.kuwo.cn/star/albumcover/500/22/47/360392367.jpg","kind":"song","source":"kuwo","artist":"刘珂矣","duration_ms":221000,"engine_hint":"bilibili"},{"id":"MUSIC_1035026","title":"雨爱","subtitle":"雨爱","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s6/93/1823888016.jpg","kind":"song","source":"kuwo","artist":"杨丞琳","duration_ms":260000,"engine_hint":"bilibili"},{"id":"MUSIC_201182467","title":"天地龙鳞","subtitle":"天地龙鳞","cover":"https://img4.kuwo.cn/star/albumcover/500/24/84/532054390.jpg","kind":"song","source":"kuwo","artist":"王力宏","duration_ms":196000,"engine_hint":"bilibili"},{"id":"MUSIC_501646","title":"奢香夫人","subtitle":"最炫民族风","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s4/17/3425185860.jpg","kind":"song","source":"kuwo","artist":"凤凰传奇","duration_ms":259000,"engine_hint":"bilibili"},{"id":"MUSIC_284892811","title":"离别开出花","subtitle":"离别开出花","cover":"https://img2.kuwo.cn/star/albumcover/500/s3s47/0/3433413293.jpg","kind":"song","source":"kuwo","artist":"就是南方凯","duration_ms":230000,"engine_hint":"bilibili"},{"id":"MUSIC_521282545","title":"街角的晚风","subtitle":"街角的晚风","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s89/8/307070993.jpg","kind":"song","source":"kuwo","artist":"陈小春","duration_ms":240000,"engine_hint":"bilibili"},{"id":"MUSIC_133360","title":"樱花草","subtitle":"花言乔语 (精装版)","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s24/5/661213926.jpg","kind":"song","source":"kuwo","artist":"Sweety","duration_ms":283000,"engine_hint":"bilibili"},{"id":"MUSIC_486366800","title":"野心家","subtitle":"灼灼韶华 影视原声带","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s40/31/3522498309.jpg","kind":"song","source":"kuwo","artist":"张靓颖","duration_ms":282000,"engine_hint":"bilibili"},{"id":"MUSIC_221822646","title":"可能","subtitle":"可能","cover":"https://img1.kuwo.cn/star/albumcover/500/53/43/1728161476.jpg","kind":"song","source":"kuwo","artist":"程响","duration_ms":217000,"engine_hint":"bilibili"},{"id":"MUSIC_93151","title":"美人鱼","subtitle":"第二天堂","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s60/86/1922632933.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":254000,"engine_hint":"bilibili"},{"id":"MUSIC_450444","title":"红色高跟鞋","subtitle":"若你碰到他","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s15/81/2985702596.jpg","kind":"song","source":"kuwo","artist":"蔡健雅","duration_ms":206000,"engine_hint":"bilibili"},{"id":"MUSIC_441426083","title":"搀扶","subtitle":"搀扶","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s60/91/4260853561.jpg","kind":"song","source":"kuwo","artist":"马健涛","duration_ms":327000,"engine_hint":"bilibili"}]';

// 酷我热评榜（bangId=284）— 抓取于 2026-08-22 00:39，rn=20 命中 19 首
const BAKED_BANG_COMMENT_RAW =
  '[{"id":"MUSIC_548659502","title":"无情的风绝情的吹(烟嗓船长版)","subtitle":"无情的风绝情的吹（烟嗓船长版）","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s44/96/2914362169.jpg","kind":"song","source":"kuwo","artist":"烟嗓船长","duration_ms":187000,"engine_hint":"bilibili"},{"id":"MUSIC_1028308","title":"他一定很爱你","subtitle":"天黑","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s6/49/3126160505.jpg","kind":"song","source":"kuwo","artist":"阿杜","duration_ms":210000,"engine_hint":"bilibili"},{"id":"MUSIC_3205111","title":"黄昏","subtitle":"戏说人生","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s77/22/3128562066.jpg","kind":"song","source":"kuwo","artist":"罗文","duration_ms":300000,"engine_hint":"bilibili"},{"id":"MUSIC_219458769","title":"三生三幸","subtitle":"三生三幸","cover":"https://img3.kuwo.cn/star/albumcover/500/34/27/3813384310.jpg","kind":"song","source":"kuwo","artist":"海来阿木","duration_ms":276000,"engine_hint":"bilibili"},{"id":"MUSIC_79503","title":"冻结","subtitle":"乐行者","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s61/16/2653293070.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":289000,"engine_hint":"bilibili"},{"id":"MUSIC_500238897","title":"岸边客","subtitle":"岸边客","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s87/88/1593565358.jpg","kind":"song","source":"kuwo","artist":"夏子航（2603）","duration_ms":272000,"engine_hint":"bilibili"},{"id":"MUSIC_228948","title":"雨一直下","subtitle":"雨一直下","cover":"https://img3.kuwo.cn/star/albumcover/500/18/42/693803121.jpg","kind":"song","source":"kuwo","artist":"张宇","duration_ms":290000,"engine_hint":"bilibili"},{"id":"MUSIC_16578753","title":"Take Me To Your Heart","subtitle":"That’s WhyYou Go Away","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s33/54/1477267688.jpg","kind":"song","source":"kuwo","artist":"Michael Learns To Rock","duration_ms":238000,"engine_hint":"bilibili"},{"id":"MUSIC_39340336","title":"ANGEL","subtitle":"Angel","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s69/69/2874934106.jpg","kind":"song","source":"kuwo","artist":"윤미래&Tiger JK&비지","duration_ms":255000,"engine_hint":"bilibili"},{"id":"MUSIC_270880103","title":"愿与愁","subtitle":"重拾_快乐","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s70/75/3885449841.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":231000,"engine_hint":"bilibili"},{"id":"MUSIC_3201831","title":"相见恨晚","subtitle":"敲敲我的头","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s33/49/4264446574.jpg","kind":"song","source":"kuwo","artist":"彭佳慧","duration_ms":254000,"engine_hint":"bilibili"},{"id":"MUSIC_156491","title":"断了的弦","subtitle":"寻找周杰伦","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s81/17/2456300215.jpg","kind":"song","source":"kuwo","artist":"周杰伦","duration_ms":297000,"engine_hint":"bilibili"},{"id":"MUSIC_158919","title":"星星点灯","subtitle":"星星点灯","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s19/21/3092357540.jpg","kind":"song","source":"kuwo","artist":"郑智化","duration_ms":302000,"engine_hint":"bilibili"},{"id":"MUSIC_150810446","title":"云与海","subtitle":"云与海","cover":"https://img4.kuwo.cn/star/albumcover/500/98/58/2902913009.jpg","kind":"song","source":"kuwo","artist":"阿YueYue","duration_ms":258000,"engine_hint":"bilibili"},{"id":"MUSIC_133520","title":"我想更懂你","subtitle":"反转地球","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s3/44/2940593875.jpg","kind":"song","source":"kuwo","artist":"潘玮柏&苏芮","duration_ms":266000,"engine_hint":"bilibili"},{"id":"MUSIC_162096535","title":"西楼别序","subtitle":"西楼别序","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s24/84/983387737.jpg","kind":"song","source":"kuwo","artist":"尹昔眠&小田音乐社","duration_ms":227000,"engine_hint":"bilibili"},{"id":"MUSIC_488172956","title":"辞九门回忆(节奏版)","subtitle":"辞九门回忆","cover":"https://img2.kuwo.cn/star/albumcover/500/45/81/3049437906.jpg","kind":"song","source":"kuwo","artist":"邓寓君(等什么君)","duration_ms":210000,"engine_hint":"bilibili"},{"id":"MUSIC_79245","title":"17岁","subtitle":"如果有一天","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s98/19/3642620202.jpg","kind":"song","source":"kuwo","artist":"刘德华","duration_ms":240000,"engine_hint":"bilibili"},{"id":"MUSIC_3208050","title":"遗失的心跳","subtitle":"SUPER GIRL 爱无畏","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s10/58/2219020403.jpg","kind":"song","source":"kuwo","artist":"萧亚轩","duration_ms":216000,"engine_hint":"bilibili"}]';

// 影视金曲榜（bangId=64）— 抓取于 2026-08-22 00:39，rn=20 命中 20 首
const BAKED_BANG_MOVIE_RAW =
  '[{"id":"MUSIC_193290598","title":"如愿","subtitle":"如愿","cover":"https://img3.kuwo.cn/star/albumcover/500/15/57/2715116001.jpg","kind":"song","source":"kuwo","artist":"王菲","duration_ms":265000,"engine_hint":"bilibili"},{"id":"MUSIC_1035026","title":"雨爱","subtitle":"雨爱","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s6/93/1823888016.jpg","kind":"song","source":"kuwo","artist":"杨丞琳","duration_ms":260000,"engine_hint":"bilibili"},{"id":"MUSIC_201182467","title":"天地龙鳞","subtitle":"天地龙鳞","cover":"https://img4.kuwo.cn/star/albumcover/500/24/84/532054390.jpg","kind":"song","source":"kuwo","artist":"王力宏","duration_ms":196000,"engine_hint":"bilibili"},{"id":"MUSIC_133360","title":"樱花草","subtitle":"花言乔语 (精装版)","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s24/5/661213926.jpg","kind":"song","source":"kuwo","artist":"Sweety","duration_ms":283000,"engine_hint":"bilibili"},{"id":"MUSIC_486366800","title":"野心家","subtitle":"灼灼韶华 影视原声带","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s40/31/3522498309.jpg","kind":"song","source":"kuwo","artist":"张靓颖","duration_ms":282000,"engine_hint":"bilibili"},{"id":"MUSIC_450444","title":"红色高跟鞋","subtitle":"若你碰到他","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s15/81/2985702596.jpg","kind":"song","source":"kuwo","artist":"蔡健雅","duration_ms":206000,"engine_hint":"bilibili"},{"id":"MUSIC_499778","title":"偏爱","subtitle":"破天荒","cover":"https://img4.kuwo.cn/star/albumcover/500/50/26/402483447.jpg","kind":"song","source":"kuwo","artist":"张芸京","duration_ms":212000,"engine_hint":"bilibili"},{"id":"MUSIC_465829","title":"情歌","subtitle":"靜茹 & 情歌 別再为他流泪","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s34/78/3678542361.jpg","kind":"song","source":"kuwo","artist":"梁静茹","duration_ms":260000,"engine_hint":"bilibili"},{"id":"MUSIC_80403","title":"十年","subtitle":"黑白灰","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s86/18/1347897214.jpg","kind":"song","source":"kuwo","artist":"陈奕迅","duration_ms":205000,"engine_hint":"bilibili"},{"id":"MUSIC_38958726","title":"后会无期","subtitle":"不良少年","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s63/58/1333131827.jpg","kind":"song","source":"kuwo","artist":"徐良&汪苏泷","duration_ms":211000,"engine_hint":"bilibili"},{"id":"MUSIC_339923255","title":"小美满","subtitle":"小美满","cover":"https://img1.kuwo.cn/star/albumcover/500/s3s40/13/2405080697.jpg","kind":"song","source":"kuwo","artist":"周深","duration_ms":214000,"engine_hint":"bilibili"},{"id":"MUSIC_39515638","title":"桃花诺","subtitle":"上古情歌 电视剧原声带","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s6/85/507888042.jpg","kind":"song","source":"kuwo","artist":"G.E.M. 邓紫棋","duration_ms":219000,"engine_hint":"bilibili"},{"id":"MUSIC_28509807","title":"无人之岛","subtitle":"没有发生的爱情","cover":"https://img3.kuwo.cn/star/albumcover/500/13/6/1934684073.jpg","kind":"song","source":"kuwo","artist":"任然","duration_ms":285000,"engine_hint":"bilibili"},{"id":"MUSIC_142801","title":"偏偏喜欢你","subtitle":"偏偏喜欢你","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s77/9/2737103598.jpg","kind":"song","source":"kuwo","artist":"陈百强","duration_ms":212000,"engine_hint":"bilibili"},{"id":"MUSIC_61997","title":"伤心太平洋","subtitle":"爱像太平洋","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s89/2/3988103284.jpg","kind":"song","source":"kuwo","artist":"任贤齐","duration_ms":266000,"engine_hint":"bilibili"},{"id":"MUSIC_55219","title":"水手","subtitle":"私房歌","cover":"https://img4.kuwo.cn/star/albumcover/500/71/11/915681638.jpg","kind":"song","source":"kuwo","artist":"郑智化","duration_ms":292000,"engine_hint":"bilibili"},{"id":"MUSIC_442546","title":"Always Online","subtitle":"JJ陆","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s61/79/3864206498.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":225000,"engine_hint":"bilibili"},{"id":"MUSIC_104595","title":"一直很安静","subtitle":"寂寞在唱歌","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s18/61/3797217633.jpg","kind":"song","source":"kuwo","artist":"阿桑","duration_ms":250000,"engine_hint":"bilibili"},{"id":"MUSIC_3620542","title":"其实","subtitle":"意外","cover":"https://img4.kuwo.cn/star/albumcover/500/39/42/1467983488.jpg","kind":"song","source":"kuwo","artist":"薛之谦","duration_ms":242000,"engine_hint":"bilibili"},{"id":"MUSIC_6844452","title":"相思","subtitle":"直接影响","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s61/78/44741830.jpg","kind":"song","source":"kuwo","artist":"毛阿敏","duration_ms":181000,"engine_hint":"bilibili"}]';

// 爆笑相声榜（bangId=291）— 抓取于 2026-08-22 00:39，rn=20 命中 20 首
const BAKED_BANG_CROSSTALK_RAW =
  '[{"id":"MUSIC_571119778","title":"No.04《卖面茶》（郭德纲 于谦）","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":1355000,"engine_hint":"bilibili"},{"id":"MUSIC_489834838","title":"大上寿-郭德纲、于谦（带返场）-01","subtitle":"郭德纲&于谦|逗笑相声合集","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s53/27/3167103010.jpg","kind":"song","source":"kuwo","artist":"郭德纲&于谦","duration_ms":242000,"engine_hint":"bilibili"},{"id":"MUSIC_382043510","title":"01.君臣斗 (一)","subtitle":"刘宝瑞相声全集|单口相声|精修版|君臣斗|官场斗|刘罗锅智斗和珅乾隆","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s67/29/2486910461.png","kind":"song","source":"kuwo","artist":"刘宝瑞","duration_ms":427000,"engine_hint":"bilibili"},{"id":"MUSIC_571119958","title":"No.03《梦中婚》（高峰 栾云平）","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":1599000,"engine_hint":"bilibili"},{"id":"MUSIC_213334807","title":"八大改行","subtitle":"郭德纲对口相声集","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s46/61/2104739032.jpg","kind":"song","source":"kuwo","artist":"郭德纲&张文顺","duration_ms":2319000,"engine_hint":"bilibili"},{"id":"MUSIC_571121579","title":"【祝你好梦】郭德纲于谦助眠专场来咯","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":2752000,"engine_hint":"bilibili"},{"id":"MUSIC_571121128","title":"No.08《托妻献子》（郭麒麟 阎鹤祥）","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":3104000,"engine_hint":"bilibili"},{"id":"MUSIC_571121067","title":"No.02《八大吉祥》（岳云鹏 孙越）","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":2226000,"engine_hint":"bilibili"},{"id":"MUSIC_571152085","title":"郭德纲x于谦《我要闹绯闻》丨揭发名人抨击古人指定能红","subtitle":"郭德纲：讽刺相声|于谦高光时刻|德云社|反焦虑|摆脱抑郁","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s42/65/3857514068.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":1631000,"engine_hint":"bilibili"},{"id":"MUSIC_338805077","title":"官场斗 01 （音质优化版）","subtitle":"刘宝瑞相声精选|经典单口","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s6/66/4026213148.jpg","kind":"song","source":"kuwo","artist":"华策影视官方","duration_ms":571000,"engine_hint":"bilibili"},{"id":"MUSIC_39948684","title":"刚好遇见你(Live)","subtitle":"2017央视元宵晚会","cover":"https://img1.kuwo.cn/star/albumcover/500/24/29/1839683929.jpg","kind":"song","source":"kuwo","artist":"李玉刚","duration_ms":188000,"engine_hint":"bilibili"},{"id":"MUSIC_571119826","title":"No.09《你得学好》（郭德纲 于谦）","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":1238000,"engine_hint":"bilibili"},{"id":"MUSIC_638289","title":"叫卖图","subtitle":"郭德纲对口相声全集","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s13/22/496155752.jpg","kind":"song","source":"kuwo","artist":"郭德纲","duration_ms":1013000,"engine_hint":"bilibili"},{"id":"MUSIC_376270532","title":"《世界那么大》曹云金 刘云天","subtitle":"曹云金相声精选集|高清爆笑","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s8/56/2617279533.png","kind":"song","source":"kuwo","artist":"曹云金","duration_ms":2013000,"engine_hint":"bilibili"},{"id":"MUSIC_2528407","title":"001珍珠翡翠白玉汤","subtitle":"刘宝瑞单口相声集","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s84/36/2343707140.jpg","kind":"song","source":"kuwo","artist":"华策影视官方","duration_ms":1954000,"engine_hint":"bilibili"},{"id":"MUSIC_4686434","title":"西征梦","subtitle":"郭德纲对口相声全集","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s13/22/496155752.jpg","kind":"song","source":"kuwo","artist":"郭德纲","duration_ms":1412000,"engine_hint":"bilibili"},{"id":"MUSIC_571144014","title":"《拍电影》| 于谦演王子，谁来演公主?","subtitle":"郭德纲经典相声|三十年巅峰佳作","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s0/59/3382765800.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":1646000,"engine_hint":"bilibili"},{"id":"MUSIC_571119509","title":"No.07《世纪名画》（郭德纲 于谦）","subtitle":"德云社相声|无唱段伴睡|郭德纲于谦领衔","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s41/39/1120723191.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":758000,"engine_hint":"bilibili"},{"id":"MUSIC_571770781","title":"《非一般的爱情》-岳云鹏","subtitle":"岳云鹏孙越相声","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s42/89/2427007744.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":943000,"engine_hint":"bilibili"},{"id":"MUSIC_571152042","title":"郭德纲x于谦《你有病啊》丨郑喜定为何改名叫郑好","subtitle":"郭德纲：讽刺相声|于谦高光时刻|德云社|反焦虑|摆脱抑郁","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s42/65/3857514068.jpg","kind":"song","source":"kuwo","artist":"喜马拉雅","duration_ms":1664000,"engine_hint":"bilibili"}]';

// 会员爱听榜（bangId=331）— 抓取于 2026-08-22 00:39，rn=20 命中 20 首
const BAKED_BANG_VIP_RAW =
  '[{"id":"MUSIC_624683929","title":"山风山风等等我","subtitle":"山风山风等等我","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s81/95/2497366108.jpg","kind":"song","source":"kuwo","artist":"万海东","duration_ms":209000,"engine_hint":"bilibili"},{"id":"MUSIC_567247828","title":"你有没有真的爱过我","subtitle":"你有没有真的爱过我","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s54/51/1818397081.jpg","kind":"song","source":"kuwo","artist":"阿图表妹","duration_ms":243000,"engine_hint":"bilibili"},{"id":"MUSIC_595898177","title":"踏马寻花向自由(雷鬼版)","subtitle":"踏马寻花向自由（雷鬼版）","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s98/67/1050454242.jpg","kind":"song","source":"kuwo","artist":"超哥","duration_ms":179000,"engine_hint":"bilibili"},{"id":"MUSIC_570482802","title":"岁月如笔写春秋","subtitle":"岁月如笔写春秋","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s80/50/3012183677.jpg","kind":"song","source":"kuwo","artist":"河南三妹5233","duration_ms":251000,"engine_hint":"bilibili"},{"id":"MUSIC_637515046","title":"好巧是你","subtitle":"好巧是你 (2026中央广播电视总台七夕晚会主题曲)","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s50/36/2192490442.jpg","kind":"song","source":"kuwo","artist":"周深","duration_ms":232000,"engine_hint":"bilibili"},{"id":"MUSIC_133360","title":"樱花草","subtitle":"花言乔语 (精装版)","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s24/5/661213926.jpg","kind":"song","source":"kuwo","artist":"Sweety","duration_ms":283000,"engine_hint":"bilibili"},{"id":"MUSIC_636218203","title":"如果累了就回故乡（少年版）","subtitle":"如果累了就回故乡","cover":"https://img2.kuwo.cn/star/albumcover/500/s4s63/2/2773368495.jpg","kind":"song","source":"kuwo","artist":"酷酷里&面妹孔佳奇","duration_ms":233000,"engine_hint":"bilibili"},{"id":"MUSIC_550221152","title":"人生路漫漫","subtitle":"人生路漫漫","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s93/35/2013522491.jpg","kind":"song","source":"kuwo","artist":"白小白","duration_ms":235000,"engine_hint":"bilibili"},{"id":"MUSIC_594551679","title":"黄昏（纵然青丝如霜）(R&B版)","subtitle":"黄昏（纵然青丝如霜）（R&B版）","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s46/81/76396974.jpg","kind":"song","source":"kuwo","artist":"落日微醺","duration_ms":245000,"engine_hint":"bilibili"},{"id":"MUSIC_583793650","title":"山歌追上云朵","subtitle":"山歌追上云朵","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s96/13/689491293.jpg","kind":"song","source":"kuwo","artist":"央金拉姆","duration_ms":215000,"engine_hint":"bilibili"},{"id":"MUSIC_22888701","title":"梦的翅膀受了伤","subtitle":"梦的翅膀受了伤","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s25/98/1876317814.jpg","kind":"song","source":"kuwo","artist":"蒋雪儿Snow.J","duration_ms":275000,"engine_hint":"bilibili"},{"id":"MUSIC_113965","title":"闭目入神","subtitle":"Before After","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s36/67/1797243847.png","kind":"song","source":"kuwo","artist":"郑中基","duration_ms":216000,"engine_hint":"bilibili"},{"id":"MUSIC_486366800","title":"野心家","subtitle":"灼灼韶华 影视原声带","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s40/31/3522498309.jpg","kind":"song","source":"kuwo","artist":"张靓颖","duration_ms":282000,"engine_hint":"bilibili"},{"id":"MUSIC_521282545","title":"街角的晚风","subtitle":"街角的晚风","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s89/8/307070993.jpg","kind":"song","source":"kuwo","artist":"陈小春","duration_ms":240000,"engine_hint":"bilibili"},{"id":"MUSIC_1044318","title":"黄昏","subtitle":"Transfer","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s49/14/2320162909.jpg","kind":"song","source":"kuwo","artist":"周传雄","duration_ms":344000,"engine_hint":"bilibili"},{"id":"MUSIC_204694228","title":"从前说","subtitle":"从前说","cover":"https://img4.kuwo.cn/star/albumcover/500/99/15/2416702591.jpg","kind":"song","source":"kuwo","artist":"小阿七","duration_ms":251000,"engine_hint":"bilibili"},{"id":"MUSIC_93151","title":"美人鱼","subtitle":"第二天堂","cover":"https://img1.kuwo.cn/star/albumcover/500/s4s60/86/1922632933.jpg","kind":"song","source":"kuwo","artist":"林俊杰","duration_ms":254000,"engine_hint":"bilibili"},{"id":"MUSIC_611237221","title":"甲乙丙丁 (你我怎么两清)","subtitle":"甲乙丙丁","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s21/72/840798623.jpg","kind":"song","source":"kuwo","artist":"李佳薇","duration_ms":210000,"engine_hint":"bilibili"},{"id":"MUSIC_632268490","title":"问情(烟嗓良人Remix)","subtitle":"问情","cover":"https://img4.kuwo.cn/star/albumcover/500/s4s13/51/1762103840.jpg","kind":"song","source":"kuwo","artist":"大头钉团队","duration_ms":256000,"engine_hint":"bilibili"},{"id":"MUSIC_637506854","title":"旧记忆","subtitle":"旧记忆","cover":"https://img3.kuwo.cn/star/albumcover/500/s4s83/48/86191593.jpg","kind":"song","source":"kuwo","artist":"黄子韬&刘宇宁","duration_ms":216000,"engine_hint":"bilibili"}]';

const BAKED_BANG_CLASSIC = (function () {
  try {
    return JSON.parse(BAKED_BANG_CLASSIC_RAW) || [];
  } catch (e) {
    return [];
  }
})();

const BAKED_BANG_HOT = (function () {
  try {
    return JSON.parse(BAKED_BANG_HOT_RAW) || [];
  } catch (e) {
    return [];
  }
})();

const BAKED_BANG_COMMENT = (function () {
  try {
    return JSON.parse(BAKED_BANG_COMMENT_RAW) || [];
  } catch (e) {
    return [];
  }
})();

const BAKED_BANG_MOVIE = (function () {
  try {
    return JSON.parse(BAKED_BANG_MOVIE_RAW) || [];
  } catch (e) {
    return [];
  }
})();

const BAKED_BANG_CROSSTALK = (function () {
  try {
    return JSON.parse(BAKED_BANG_CROSSTALK_RAW) || [];
  } catch (e) {
    return [];
  }
})();

const BAKED_BANG_VIP = (function () {
  try {
    return JSON.parse(BAKED_BANG_VIP_RAW) || [];
  } catch (e) {
    return [];
  }
})();

// 酷我新歌榜（bangId=17）的 baked snapshot —— 抓取于 2026-08-22 18:51，沙箱 IP 还通。
// runtime 优先 + baked 兜底，与 BANG_HOT 等同模式。当 BANG_SECRET/Cookie 旋转时
// 自动退回这里，避免「新歌首发」区块为空。
const BAKED_BANG_NEW_RAW =
  '[{"rid":628379347,"name":"金达莱花(猛Remix)","artist":"猛弟/邹子寒","album":"진달래꽃 金达莱花(猛Remix）","pic":"https://img4.kuwo.cn/star/albumcover/500/s4s7/80/882127986.jpg","duration":138},{"rid":637903543,"name":"彻底的沦陷(Live)","artist":"TOP登陆少年-张极","album":"2026年TOP登陆少年组合‘浪漫主义’系列演唱会——「梦寐以求」Day3&Day4 LIVE音频","pic":"https://img3.kuwo.cn/star/albumcover/500/s4s89/11/2857229249.jpg","duration":241},{"rid":627074689,"name":"汹涌汹涌","artist":"段奕帆","album":"汹涌汹涌","pic":"https://img3.kuwo.cn/star/albumcover/500/s4s87/51/2671381337.jpg","duration":171},{"rid":637515046,"name":"好巧是你","artist":"周深","album":"好巧是你 (2026中央广播电视总台七夕晚会主题曲)","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s50/36/2192490442.jpg","duration":232},{"rid":636218203,"name":"如果累了就回故乡（少年版）","artist":"酷酷里&面妹孔佳奇","album":"如果累了就回故乡","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s63/2/2773368495.jpg","duration":233},{"rid":626580509,"name":"白玉染尘","artist":"武怀琛","album":"云中客","pic":"https://img4.kuwo.cn/star/albumcover/500/s4s51/81/2115076542.jpg","duration":303},{"rid":636547349,"name":"RUN IT","artist":"梓渝","album":"梓渝","pic":"https://img3.kuwo.cn/star/albumcover/500/s4s20/30/2271164161.jpg","duration":197},{"rid":630962031,"name":"每个人(舒楠监制 官方正式版)","artist":"周深","album":"每个人 (舒楠监制 官方正式版)","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s57/54/353112138.jpg","duration":231},{"rid":631458551,"name":"借酒","artist":"张大迷糊","album":"借酒","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s5/7/4096054922.jpg","duration":182},{"rid":627191779,"name":"如约","artist":"周深","album":"如约","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s82/92/3231984954.jpg","duration":236},{"rid":631912543,"name":"梦的翅膀受了伤(一滴一滴刺痛我的心)","artist":"DJ铁柱&蓝心雨","album":"梦的翅膀受了伤（一滴一滴刺痛我的心）","pic":"https://img3.kuwo.cn/star/albumcover/500/s4s29/32/2472358221.jpg","duration":114},{"rid":636229636,"name":"没关系(Live)","artist":"周深","album":"国乐无双 第12期","pic":"https://img4.kuwo.cn/star/albumcover/500/s4s2/27/2644417858.jpg","duration":216},{"rid":637506854,"name":"旧记忆","artist":"黄子韬&刘宇宁","album":"旧记忆","pic":"https://img3.kuwo.cn/star/albumcover/500/s4s83/48/86191593.jpg","duration":216},{"rid":637418433,"name":"想你了","artist":"TF_ING邓佳鑫","album":"想你了","pic":"https://img4.kuwo.cn/star/albumcover/500/s4s51/49/1338148949.jpg","duration":202},{"rid":637324505,"name":"洞房","artist":"张云雷","album":"洞房","pic":"https://img1.kuwo.cn/star/albumcover/500/s4s67/47/1241867148.jpg","duration":226},{"rid":627897741,"name":"今生劫换来生缘","artist":"黄静美","album":"今生劫换来生缘","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s74/25/3794865950.jpg","duration":225},{"rid":637398323,"name":"晚风告白","artist":"黄星&邱鼎杰","album":"晚风知我意","pic":"https://img2.kuwo.cn/star/albumcover/500/s4s42/38/2888780200.jpg","duration":237},{"rid":635050968,"name":"人间藏欢喜","artist":"孙红雷&李乃文&郭京飞&刘宇宁&龚俊&陈星旭&王玉雯&林一","album":"人间藏欢喜","pic":"https://img1.kuwo.cn/star/albumcover/500/s4s80/16/3362200418.jpg","duration":180},{"rid":632588035,"name":"今生劫换来生缘(串烧版)","artist":"贰婶子","album":"今生劫换来生缘（串烧版）","pic":"https://img1.kuwo.cn/star/albumcover/500/s4s54/99/3434692559.jpg","duration":152},{"rid":637010989,"name":"就当你是个过客(串烧版)","artist":"L（桃籽）&杨不乖&万能和弦","album":"就当你是个过客（串烧版）","pic":"https://img4.kuwo.cn/star/albumcover/500/s4s87/19/3201015986.jpg","duration":244}]';
const BAKED_BANG_NEW = (function () {
  try {
    return JSON.parse(BAKED_BANG_NEW_RAW) || [];
  } catch (e) {
    return [];
  }
})();
// payload bytes=3252 items=20

// 把 bakedKey 映射到 BAKED_BANG_<KEY>。key 不识别或 baked 数组为空时返回 null，
// 调用方应在 null 时跳过该区块（运行时优先 + baked 同时失败的情况）。
function lookupBakedBang(key) {
  switch (key) {
    case 'CLASSIC':
      return BAKED_BANG_CLASSIC;
    case 'HOT':
      return BAKED_BANG_HOT;
    case 'COMMENT':
      return BAKED_BANG_COMMENT;
    case 'MOVIE':
      return BAKED_BANG_MOVIE;
    case 'CROSSTALK':
      return BAKED_BANG_CROSSTALK;
    case 'VIP':
      return BAKED_BANG_VIP;
    case 'NEW':
      return BAKED_BANG_NEW;
    default:
      return null;
  }
}

function webHeaders(query) {
  const ref = REFERER_BASE + encodeURIComponent(query || '');
  return {
    'User-Agent': DESKTOP_UA,
    Accept: 'application/json, text/plain, */*',
    'Accept-Language': 'zh-CN,zh;q=0.9,en;q=0.8',
    'X-Requested-With': 'XMLHttpRequest',
    Origin: 'https://www.kuwo.cn',
    Referer: ref,
    Connection: 'keep-alive',
  };
}

// --- utilities -------------------------------------------------------------

function reqId() {
  let out = '';
  const HEX = '0123456789abcdef';
  for (let i = 0; i < 36; i++) {
    if (i === 8 || i === 13 || i === 18 || i === 23) {
      out += '-';
    } else if (i === 14) {
      out += '4';
    } else if (i === 19) {
      out += HEX[Math.floor(Math.random() * 4) + 8];
    } else {
      out += HEX[Math.floor(Math.random() * 16)];
    }
  }
  return out;
}

function httpGet(url, headers) {
  return bilusic.httpGet(url, JSON.stringify({ headers: headers || {} }));
}

function unwrapJsonp(text) {
  if (!text) return text;
  const trimmed = text.trim();
  if (trimmed.charAt(0) === '{' || trimmed.charAt(0) === '[') {
    return trimmed;
  }

  const open = trimmed.indexOf('(');
  const close = trimmed.lastIndexOf(')');
  if (open >= 0 && close > open) {
    return trimmed.substring(open + 1, close);
  }
  return trimmed;
}

function parseDurationSeconds(value) {
  if (value == null) return 0;
  if (typeof value === 'number') return Math.max(0, value);
  const s = String(value).trim();
  if (s.indexOf(':') >= 0) {
    const parts = s.split(':');
    let sec = 0;
    for (let i = 0; i < parts.length; i++) {
      sec = sec * 60 + (parseInt(parts[i], 10) || 0);
    }
    return sec;
  }
  const n = parseInt(s, 10);
  return Number.isFinite(n) ? Math.max(0, n) : 0;
}

function ensureSourceId(id) {
  if (!id) return '';
  return String(id);
}

function songlistToTracks(songlist, sourceHint) {
  if (!Array.isArray(songlist)) return [];
  const out = [];
  for (const s of songlist) {
    if (!s) continue;

    let id;
    if (s.rid) id = 'MUSIC_' + s.rid;
    else id = String(s.MUSICRID || s.musicrid || s.id || '');
    if (!id) continue;
    // Cover ladder: the new `/search/searchMusicBykeyWord` endpoint returns
    // UPPERCASE keys, with `hts_MVPIC` being the only full-URL form
    // (`https://imgN.kuwo.cn/wmvpic/324/...jpg`). Lowercase `albumpic`
    // is the legacy `/api/www/...` shape. Try high-signal full-URL
    // sources first, then dash-prefixed uppercase forms, then the
    // bare-relative paths (which need a domain stitched on, so they're
    // deprioritised).
    const cover = String(
      s.hts_MVPIC ||
        s.hts_mvpic ||
        s.MVPIC ||
        s.HTS_MVPIC ||
        s.WEB_ALBUMPIC ||
        s.web_albumpic ||
        s.ALBUMPIC ||
        s.albumpic ||
        s.albumpic_small ||
        s.pic ||
        s.PIC ||
        s.album_pic ||
        '',
    );
    const track = {
      source_id: id,
      source: sourceHint || 'kuwo',
      engine_hint: 'bilibili',
      title: String(s.name || s.NAME || '').trim(),
      artist: String(s.artist || s.ARTIST || '').trim(),
      album: String(s.album || s.ALBUM || '').trim(),
      duration_ms: parseDurationSeconds(s.duration || s.DURATION) * 1000,
      cover: cover,
      lyrics_id: null,
    };
    if (!track.title) continue;
    out.push(track);
  }
  return out;
}

function pickSongList(data) {
  if (!data || typeof data !== 'object') return [];
  const paths = [
    ['data', 'abslist'],
    ['data', 'songs'],
    ['data', 'list'],
    ['abslist'],
    ['songs'],
    ['list'],
    ['data', 'musicList'],
    ['musiclist'],
  ];
  for (let i = 0; i < paths.length; i++) {
    const path = paths[i];
    let cur = data;
    let found = true;
    for (let j = 0; j < path.length; j++) {
      const key = path[j];
      if (cur == null || typeof cur !== 'object') {
        found = false;
        break;
      }
      cur = cur[key];
    }
    if (found && Array.isArray(cur) && cur.length > 0) return cur;
  }
  // Final fallback: if the body is itself an array, use it directly.
  if (Array.isArray(data)) return data;
  return [];
}

// 30-second in-memory cache for home so navigating in/out doesn't refetch.
const HOME_TTL_MS = 30000;
const _homeCache = { at: 0, payload: null };
function cachedHome() {
  if (_homeCache.payload && Date.now() - _homeCache.at < HOME_TTL_MS) {
    return _homeCache.payload;
  }
  return null;
}
function setHomeCache(payload) {
  _homeCache.at = Date.now();
  _homeCache.payload = payload;
}

// --- endpoints -------------------------------------------------------------
const SEARCH_ENDPOINT =
  'https://www.kuwo.cn/search/searchMusicBykeyWord?' +
  'vipver=1&client=kt&ft=music&cluster=0&strategy=2012' +
  '&encoding=utf8&rformat=json&mobi=1&issubtitle=1' +
  '&show_copyright_off=1';

function searchOp(query, page) {
  const p = page || 1;
  const enc = encodeURIComponent(query || '');
  const url =
    SEARCH_ENDPOINT + '&pn=' + encodeURIComponent(p) + '&rn=20&all=' + enc;
  let resp;
  try {
    resp = httpGet(url, webHeaders(query));
  } catch (e) {
    bilusic.log('warn', '[kuwo][search] httpGet threw: ' + (e && e.message));
    return { tracks: [] };
  }
  if (!resp || typeof resp !== 'object' || !resp.ok) {
    return { tracks: [] };
  }
  let data;
  try {
    const body = resp.body;
    if (body == null || body === '') {
      return { tracks: [] };
    }
    data = JSON.parse(body);
  } catch (e) {
    bilusic.log(
      'warn',
      '[kuwo][search] JSON parse failed: ' + (e && e.message),
    );
    return { tracks: [] };
  }

  if (data && data.success === false) {
    return { tracks: [] };
  }
  if (data && typeof data.status !== 'undefined' && data.status === 0) {
    return { tracks: [] };
  }

  const listRaw = pickSongList(data);
  const songlist = Array.isArray(listRaw) ? listRaw : [];
  const tracks = songlistToTracks(songlist, 'kuwo');
  return { tracks: tracks };
}

function pickArtistList(data) {
  if (!data) return [];
  if (Array.isArray(data)) return data;
  if (data.data && Array.isArray(data.data.artistList))
    return data.data.artistList;
  if (Array.isArray(data.artistList)) return data.artistList;
  return [];
}

function artistReqId() {
  function rand4() {
    const HEX = '0123456789abcdef';
    let s = '';
    for (let i = 0; i < 4; i++) s += HEX[Math.floor(Math.random() * 16)];
    return s;
  }
  return (
    rand4() +
    rand4() +
    '-' +
    rand4() +
    '-' +
    '4' +
    rand4().slice(1) +
    '-' + // version 4 hint
    rand4() +
    '-' +
    rand4() +
    rand4() +
    rand4()
  );
}

function artistInfoHeaders() {
  return {
    'User-Agent': ARTIST_UA,
    Accept: 'application/json, text/plain, */*',
    'Accept-Language': 'zh-CN,zh;q=0.5',
    'Cache-Control': 'no-cache',
    Cookie: ARTIST_COOKIE,
    Pragma: 'no-cache',
    'Proxy-Connection': 'keep-alive',
    Referer: ARTIST_REFERER,
    'Sec-GPC': '1',
    Secret: ARTIST_SECRET,
  };
}

function artistInfoOp() {
  const url = ARTIST_INFO_URL + artistReqId();
  let resp;
  try {
    resp = httpGet(url, artistInfoHeaders());
  } catch (e) {
    bilusic.log(
      'warn',
      '[kuwo][artistInfo] httpGet threw: ' +
        (e && e.message ? e.message : 'unknown') +
        ' → fallback BAKED_ARTISTS',
    );
    return null;
  }
  if (!resp || typeof resp !== 'object' || !resp.ok) {
    return null;
  }
  const body = resp.body;
  if (!body || body === '') {
    return null;
  }
  let data;
  try {
    data = JSON.parse(body);
  } catch (e) {
    bilusic.log(
      'warn',
      '[kuwo][artistInfo] JSON parse failed: ' +
        (e && e.message ? e.message : 'unknown') +
        ' → fallback BAKED_ARTISTS',
    );
    return null;
  }
  if (data && data.success === false) {
    return null;
  }
  if (data && data.code !== undefined && data.code !== 200 && data.code !== 0) {
    return null;
  }
  const list = pickArtistList(data);
  if (!list || list.length === 0) {
    return null;
  }
  // Sanity: drop items missing id or name so the renderer never blanks
  // out a row. (Kuwo occasionally returns partial rows for retired
  // artists.)
  const cleaned = [];
  for (let i = 0; i < list.length; i++) {
    if (list[i] && list[i].id && list[i].name) cleaned.push(list[i]);
  }
  if (cleaned.length === 0) {
    return null;
  }
  return cleaned;
}

function getTrackOp(id) {
  const emptyTrack = function (sid) {
    return {
      track: {
        source_id: String(sid || ''),
        source: 'kuwo',
        engine_hint: 'bilibili',
        title: '',
        artist: '',
        album: '',
        duration_ms: 0,
        cover: '',
        lyrics_id: String(sid || ''),
      },
    };
  };
  if (!id) {
    return emptyTrack('');
  }
  const numeric = String(id).replace(/^MUSIC_/, '');

  const url =
    'https://www.kuwo.cn/api/www/music/musicInfo?mid=' +
    encodeURIComponent(numeric) +
    '&httpsStatus=1';
  const resp = httpGet(url, webHeaders(''));
  if (resp && resp.ok) {
    try {
      const data = JSON.parse(resp.body);
      if (!(data && data.success === false)) {
        const info = (data && data.data) || {};
        const tracks = songlistToTracks([info], 'kuwo');
        const track = tracks[0];
        if (track && track.title) {
          // Carry the rid back as lyrics_id so get_lyrics can reuse it.
          if (!track.lyrics_id) {
            track.lyrics_id = info.rid ? 'MUSIC_' + info.rid : String(id);
          }
          return { track: track };
        }
      }
    } catch (e) {
      bilusic.log(
        'warn',
        '[kuwo][get_track] musicInfo parse failed: ' + e.message,
      );
    }
  }
  // Fallback: search by id via the (now web) search endpoint.
  const fb = searchOp(String(id), 1);
  const t = (fb.tracks || [])[0];
  if (t) {
    if (!t.lyrics_id) t.lyrics_id = String(id);
    return { track: t };
  }
  return emptyTrack(id);
}

function getLyricsOp(track) {
  const id = (track && (track.lyrics_id || track.source_id)) || '';
  if (!id) return { lyrics: { lines: [] } };
  const numeric = String(id).replace(/^MUSIC_/, '');
  const url =
    'https://www.kuwo.cn/api/v1/www/lyric/getLyricByLine?' +
    'musicId=' +
    encodeURIComponent(numeric) +
    '&httpsStatus=1';
  // Lyric endpoint. Same anonymous-header treatment as search/musicInfo.
  const resp = httpGet(url, webHeaders(''));
  if (!resp || !resp.ok) {
    return { lyrics: { lines: [] } };
  }
  let data;
  try {
    data = JSON.parse(resp.body);
  } catch (e) {
    bilusic.log(
      'warn',
      '[kuwo][get_lyrics] JSON parse failed: ' +
        e.message +
        ' head=' +
        (resp.body || '').slice(0, 200),
    );
    return { lyrics: { lines: [] } };
  }
  if (data && data.success === false) {
    return { lyrics: { lines: [] } };
  }
  const lrclist =
    (data &&
      ((data.data && data.data.lrclist) ||
        data.lrclist ||
        (data.result && data.result.lrclist))) ||
    [];
  if (!Array.isArray(lrclist)) return { lyrics: { lines: [] } };
  const lines = [];
  for (const item of lrclist) {
    const text = item.lineLyric || item.line || item.text || item.content || '';
    if (typeof text !== 'string' || !text.trim()) continue;
    let timeMs = 0;
    let raw = null;
    const timeKeys = ['time', 't', 'startTime'];
    for (let ki = 0; ki < timeKeys.length; ki++) {
      const k = timeKeys[ki];
      if (typeof item[k] !== 'undefined') {
        raw = item[k];
        break;
      }
    }
    if (typeof raw === 'number') {
      timeMs = Math.round(raw < 6000 ? raw * 1000 : raw);
    } else if (typeof raw === 'string') {
      const n = parseFloat(raw);
      if (Number.isFinite(n)) timeMs = Math.round(n < 6000 ? n * 1000 : n);
    }
    lines.push({ time_ms: timeMs, text: text });
  }
  return { lyrics: { lines: lines } };
}

// 拉取单个酷我榜单的歌曲列表（musicList 接口）。
// 不同 bangId 对应不同榜单（经典怀旧=26 / 热歌=16 / 热评=284 / 影视金曲=64 / 相声=291 / 会员爱听=331）。
// 接口需要 Secret + Cookie + Chrome151 UA 三件套，与 artistInfo 同源。
// 任意失败（HTTP/解析/空列表/code!=200）返回 null，由 homeOp 决定跳过该区块。
function bangMusicListOp(bangId, title) {
  const url =
    BANG_MUSIC_LIST_URL +
    'bangId=' +
    encodeURIComponent(String(bangId)) +
    '&pn=1&rn=20&reqId=' +
    reqId();
  const headers = {
    'User-Agent': BANG_UA,
    Accept: 'application/json, text/plain, */*',
    'Accept-Language': 'zh-CN,zh;q=0.5',
    'Cache-Control': 'no-cache',
    Pragma: 'no-cache',
    'Proxy-Connection': 'keep-alive',
    Referer: BANG_REFERER,
    'Sec-GPC': '1',
    Secret: BANG_SECRET,
    Cookie: BANG_COOKIE,
  };
  let resp;
  try {
    resp = httpGet(url, headers);
  } catch (e) {
    return null;
  }
  if (!resp || !resp.ok) {
    return null;
  }
  let data;
  try {
    data = JSON.parse(resp.body);
  } catch (e) {
    bilusic.log(
      'warn',
      '[kuwo][bang] ' + title + ' JSON parse failed: ' + e.message,
    );
    return null;
  }
  // kuwo 反爬时返回 {"success":false,"message":"The request is illegal!"}，
  // 但我们仍把 body 前 160 字符 dump 出来便于排查风控/签名变化。
  if (data && data.success === false) {
    return null;
  }
  if (!data || data.code !== 200) {
    const code = data ? data.code : 'no-data';
    const msg = data && data.message ? ' msg=' + data.message : '';
    return null;
  }
  const list = (data && data.data && data.data.musicList) || [];
  if (list.length === 0) {
    return null;
  }
  const items = list
    .map(function (s) {
      const rid =
        s.rid || (s.musicrid ? String(s.musicrid).replace('MUSIC_', '') : null);
      if (!rid) return null;
      return {
        id: 'MUSIC_' + rid,
        title: s.name || s.title || s.NAME || '',
        subtitle: s.album || s.ALBUM || null,
        cover: String(
          s.hts_MVPIC ||
            s.HTS_MVPIC ||
            s.MVPIC ||
            s.WEB_ALBUMPIC ||
            s.web_albumpic ||
            s.pic ||
            s.albumpic ||
            s.pic120 ||
            s.PIC120 ||
            '',
        ),
        kind: 'song',
        source: 'kuwo',
        artist: s.artist || s.ARTIST || null,
        duration_ms: typeof s.duration === 'number' ? s.duration * 1000 : null,
        engine_hint: 'bilibili',
      };
    })
    .filter(function (it) {
      return it && it.title;
    });
  if (items.length === 0) {
    return null;
  }
  return items;
}

function homeOp() {
  const cached = cachedHome();
  if (cached) {
    return cached;
  }
  // 「新歌首发」：用 bangId=17 的 musicList 接口（与 bang 区块同模式）：
  // 「酷我新歌」榜是 kuwo 自身的官方榜（搜「全网新歌榜」关键词会得到一堆
  // 不相关的内容排序靠前，例如相关图书/有声书。所以必须走 musicList+bangId=17）。
  // 运行时优先 + baked 兜底：BANG_SECRET/Cookie 旋转时退回 BAKED_BANG_NEW。
  let newSongs = [];
  let newSongsSource = 'live';
  try {
    const live = bangMusicListOp('17', '新歌首发');
    if (live && live.length > 0) {
      newSongs = live;
    } else {
      const baked = lookupBakedBang('NEW');
      if (baked && baked.length > 0) {
        newSongs = baked.map((s) => ({
          id: 'MUSIC_' + s.rid,
          title: s.name,
          subtitle: s.album,
          cover: s.pic,
          kind: 'song',
          source: 'kuwo',
          artist: s.artist,
          duration_ms: s.duration * 1000,
          engine_hint: 'bilibili',
        }));
        newSongsSource = 'baked';
        // Quietly swap to baked fallback — the matching runtime error
        // (e.g. "[kuwo][bang] 新歌首发 code=-1 ...") is already logged inside
        // `bangMusicListOp`. A second "using baked fallback" line per bang
        // would just be noise on the home page.
      }
    }
  } catch (e) {
    bilusic.log('warn', '[kuwo][home] newSongs threw: ' + e.message);
    newSongs = [];
  }

  let artists;
  let artistsDataSource = 'baked';
  try {
    const live = artistInfoOp();
    if (live && live.length > 0) {
      artists = live.map((a) => ({
        id: 'kuwo-artist-' + a.id,
        title: a.name || '',
        subtitle: a.aartist || null,
        cover: a.pic300 || a.pic || '',
        kind: 'artist',
        source: 'kuwo',
      }));
      artistsDataSource = 'live';
    } else {
      artists = BAKED_ARTISTS.map((a) => ({
        id: 'kuwo-artist-' + a.id,
        title: a.title,
        subtitle: a.subtitle,
        cover: a.cover,
        kind: 'artist',
        source: 'kuwo',
      }));
    }
  } catch (e) {
    artists = BAKED_ARTISTS.map((a) => ({
      id: 'kuwo-artist-' + a.id,
      title: a.title,
      subtitle: a.subtitle,
      cover: a.cover,
      kind: 'artist',
      source: 'kuwo',
    }));
  }

  // 组装声明式 sections（home-architecture 改造）：
  //  · 新歌首发（song，可播放，bangId=17 musicList）
  //  · 热门歌手（card，artist 瓦片，运行时优先 + baked 兜底）
  //  · 6 个特色榜单（song，每个 bangId 一个区块）
  // 前端按 section.kind 路由渲染，标题由插件给出。
  const sections = [];
  if (newSongs.length > 0) {
    const newHint = newSongsSource === 'baked' ? '酷我热歌' : '酷我实时热歌';
    sections.push({
      id: 'kw_newsongs',
      title: '新歌首发',
      kind: 'song',
      hint: newHint,
      items: newSongs,
    });
  }

  // 特色榜单（song，可播放）：每个 bangId 一个区块，显示在「热门歌手」之上。
  // 顺序即展示顺序。单榜失败不影响其他区块。
  //
  // 运行时优先：先调 live API（带 BANG 三件套），失败/空回退到 baked snapshot。
  // 与 homeOp 的 artists 区段同模式：BANG_SECRET/Cookie 旋转失效时，
  // 首页不至于空白（这是 N11n 时已经预言的退化路径）。
  //
  // bakedKey 与 BANG_KINDS 表对应，映射到 BAKED_BANG_<KEY> IIFE 已解析数组。
  const bangSpecs = [
    ['26', 'kw_bang_classic', '经典怀旧榜', 'CLASSIC'],
    ['16', 'kw_bang_hot', '酷我热歌榜', 'HOT'],
    ['284', 'kw_bang_comment', '酷我热评榜', 'COMMENT'],
    ['64', 'kw_bang_movie', '影视金曲榜', 'MOVIE'],
    ['291', 'kw_bang_crosstalk', '爆笑相声榜', 'CROSSTALK'],
    ['331', 'kw_bang_vip', '会员爱听榜', 'VIP'],
  ];
  for (let i = 0; i < bangSpecs.length; i++) {
    const spec = bangSpecs[i];
    const bangId = spec[0];
    const sid = spec[1];
    const title = spec[2];
    const bakedKey = spec[3];
    let live = null;
    try {
      live = bangMusicListOp(bangId, title);
    } catch (e) {
      bilusic.log(
        'warn',
        '[kuwo][home] bang "' + title + '" threw: ' + e.message,
      );
    }
    let items = live && live.length > 0 ? live : null;
    let dataSource = 'live';
    if (!items) {
      const baked = lookupBakedBang(bakedKey);
      if (baked && baked.length > 0) {
        items = baked;
        dataSource = 'baked';
        // Same rationale as newSongs below — the underlying runtime error
        // was already surfaced by `bangMusicListOp`'s `[kuwo][bang]` warn;
        // logging "using baked fallback" here would repeat it for every
        // bang (6× per home load when Secret rotation / IP throttling
        // knocks them all out at once). Stay quiet and let baked serve.
      }
    }
    if (items && items.length > 0) {
      const hintSuffix = dataSource === 'baked' ? '' : '榜单分类';
      sections.push({
        id: sid,
        title: title,
        kind: 'song',
        hint: hintSuffix,
        items: items,
      });
    }
  }

  if (artists.length > 0) {
    sections.push({
      id: 'kw_artists',
      title: '热门歌手',
      kind: 'card',
      hint: '点击跳转搜索',
      items: artists,
    });
  }

  const payload = {
    home: {
      sections: sections,
    },
  };
  setHomeCache(payload);
  return payload;
}

function toplistOp(topid) {
  const queries = {
    1: '热歌榜',
    2: '新歌榜',
    3: '飙升榜',
    4: '电音榜',
    5: 'ACG榜',
    6: '粤语榜',
    7: '欧美榜',
    8: '韩语榜',
    9: '日语榜',
  };
  const query = queries[topid] || '热门歌曲';

  const out = searchOp(query, 1);
  const tracks = out.tracks || [];
  const songs = tracks.map((t) => ({ track: t, reason: 'bang:' + query }));
  return { songs: songs };
}

// --- entrypoint ------------------------------------------------------------
function main(inputJson) {
  var input = {};
  try {
    input = JSON.parse(typeof inputJson === 'string' ? inputJson : '{}');
  } catch (e) {
    input = {};
  }
  function out(obj) {
    try {
      return JSON.stringify(obj == null ? {} : obj);
    } catch (e) {
      return '{}';
    }
  }
  function safe(opName, fallback, fn) {
    try {
      var v = fn();
      if (v == null) return out(fallback);
      return out(v);
    } catch (e) {
      return out(fallback);
    }
  }
  try {
    var op = input.op;
    // Legacy path (no `op` field) — kept for Phase III plugin compatibility.
    if (op == null || op === '') {
      if (input.query !== undefined) {
        return safe('search', { tracks: [] }, function () {
          return searchOp(input.query, input.page || 1);
        });
      }
      if (input.id !== undefined) {
        return safe('get_track', { track: null }, function () {
          return getTrackOp(input.id);
        });
      }
      return '{}';
    }
    switch (op) {
      case 'search':
        return safe('search', { tracks: [] }, function () {
          return searchOp(input.query, input.page || 1);
        });
      case 'get_track':
        return safe('get_track', { track: null }, function () {
          return getTrackOp(input.id);
        });
      case 'get_lyrics':
        return safe('get_lyrics', { lyrics: { lines: [] } }, function () {
          return getLyricsOp(input.track || {});
        });
      case 'home':
        return safe(
          'home',
          {
            home: { playlists: [], new_songs: [], new_albums: [], artists: [] },
          },
          function () {
            return homeOp();
          },
        );
      case 'toplist':
        return safe('toplist', { songs: [] }, function () {
          return toplistOp(Number(input.topid) || 0);
        });
      default:
        return '{}';
    }
  } catch (e) {
    return '{}';
  }
}
