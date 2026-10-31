use std::path::Path;

use serde_json::{json, Value};

const INNERTUBE_API_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const ANDROID_API_KEY: &str = "AIzaSyA8eiZmM1FaDVjRy-df2KTyQ_vz_yYM39w";
const SEARCH_URL: &str = "https://music.youtube.com/youtubei/v1/search";
const PLAYER_URL: &str = "https://www.youtube.com/youtubei/v1/player";

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrackResult {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration: String,
    pub duration_ms: u64,
    pub thumbnail: Option<String>,
    pub artist_id: Option<String>,
    pub album_id: Option<String>,
    pub album: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShelfItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ArtistPage {
    pub name: String,
    pub thumbnail: Option<String>,
    pub top_songs: Vec<TrackResult>,
    pub albums: Vec<ShelfItem>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AlbumPage {
    pub title: String,
    pub artist: String,
    pub thumbnail: Option<String>,
    pub tracks: Vec<TrackResult>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LyricsResult {
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LyricsSearchResult {
    duration: f64,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

pub struct Innertube {
    http: reqwest::Client,
}

impl Innertube {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent("com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip")
            .build()
            .expect("failed to build http client");
        Self { http }
    }

    fn web_remix_context() -> Value {
        json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20241021.01.00",
                "hl": "en",
                "gl": "US"
            }
        })
    }

    /// Search YouTube Music. Merges the "songs" and "videos" scopes so
    /// community uploads that lack a song entity are still found.
    pub async fn search(&self, query: &str) -> Result<Vec<TrackResult>, String> {
        // ytmusicapi filter params: songs, then videos
        const SONGS: &str = "EgWKAQIIAWoKEAoQCRADEAQQCRAF";
        const VIDEOS: &str = "EgWKAQIQAWoKEAoQCRADEAQQCRAF";

        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut last_error = String::new();
        for params in [SONGS, VIDEOS] {
            let body = json!({
                "context": Self::web_remix_context(),
                "query": query,
                "params": params
            });
            let resp: Value = match self
                .http
                .post(format!("{SEARCH_URL}?key={INNERTUBE_API_KEY}"))
                .json(&body)
                .send()
                .await
            {
                Ok(response) => match response.json().await {
                    Ok(value) => value,
                    Err(e) => {
                        last_error = format!("invalid search response: {e}");
                        continue;
                    }
                },
                Err(e) => {
                    last_error = format!("search request failed: {e}");
                    continue;
                }
            };
            for track in parse_search_results(&resp) {
                if seen.insert(track.video_id.clone()) {
                    results.push(track);
                }
            }
        }

        if results.is_empty() && !last_error.is_empty() {
            return Err(last_error);
        }
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|token| token.to_lowercase())
            .collect();
        let mut scored: Vec<(u32, TrackResult)> = results
            .into_iter()
            .map(|track| (relevance_score(&track, &tokens), track))
            .collect();
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        results = scored.into_iter().map(|(_, track)| track).collect();
        results.truncate(40);
        Ok(results)
    }

    /// Up-next queue for a track via the `RDAMVM` automix playlist — the
    /// same data YouTube Music uses for endless radio.
    pub async fn radio(&self, video_id: &str) -> Result<Vec<TrackResult>, String> {
        let body = json!({
            "context": Self::web_remix_context(),
            "videoId": video_id,
            "playlistId": format!("RDAMVM{video_id}"),
            "enablePersistentPlaylistPanel": true,
            "isAudioOnly": true,
        });
        let response: Value = self
            .http
            .post(format!(
                "https://music.youtube.com/youtubei/v1/next?key={INNERTUBE_API_KEY}"
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("radio request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("invalid radio response: {e}"))?;

        let Some(panel) = find_key(&response, "playlistPanelRenderer") else {
            return Err("YouTube Music returned no radio queue".to_string());
        };
        let items = panel
            .get("contents")
            .and_then(Value::as_array)
            .ok_or_else(|| "radio queue is empty".to_string())?;
        let mut tracks = Vec::new();
        for item in items {
            let Some(r) = item.get("playlistPanelVideoRenderer") else {
                continue;
            };
            let Some(video_id) = r.get("videoId").and_then(Value::as_str) else {
                continue;
            };
            if r.pointer("/selected").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let title = r
                .pointer("/title/runs/0/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let artist = r
                .pointer("/longBylineText/runs")
                .and_then(Value::as_array)
                .map(|runs| {
                    runs.iter()
                        .filter_map(|run| run.get("text").and_then(Value::as_str))
                        .find(|text| *text != " • " && !text.starts_with("•"))
                        .unwrap_or_default()
                        .to_string()
                })
                .unwrap_or_default();
            let duration = r
                .pointer("/lengthText/runs/0/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let thumbnail = r
                .pointer("/thumbnail/thumbnails")
                .and_then(Value::as_array)
                .and_then(|thumbs| thumbs.last())
                .and_then(|thumb| thumb.get("url"))
                .and_then(Value::as_str)
                .map(String::from);
            tracks.push(TrackResult {
                video_id: video_id.to_string(),
                title,
                artist,
                duration_ms: parse_duration_ms(&duration),
                duration,
                thumbnail,
                artist_id: None,
                album_id: None,
                album: None,
            });
        }
        if tracks.is_empty() {
            return Err("YouTube Music returned no radio tracks".to_string());
        }
        tracks.truncate(25);
        Ok(tracks)
    }

    /// Anonymous Innertube browse for public artist/album pages.
    async fn browse(&self, browse_id: &str) -> Result<Value, String> {
        self.http
            .post(format!(
                "https://music.youtube.com/youtubei/v1/browse?key={INNERTUBE_API_KEY}"
            ))
            .json(&json!({
                "context": Self::web_remix_context(),
                "browseId": browse_id
            }))
            .send()
            .await
            .map_err(|e| format!("browse request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("invalid browse response: {e}"))
    }

    /// Artist page: top songs shelf plus album/single shelves.
    pub async fn artist_page(&self, browse_id: &str) -> Result<ArtistPage, String> {
        let response = self.browse(browse_id).await?;
        let header = find_key(&response, "musicImmersiveHeaderRenderer")
            .or_else(|| find_key(&response, "musicVisualHeaderRenderer"))
            .or_else(|| find_key(&response, "musicDetailHeaderRenderer"));
        let name = header
            .and_then(|h| h.pointer("/title/runs/0/text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let thumbnail = header
            .and_then(|h| {
                h.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
                    .or_else(|| h.pointer("/thumbnail/thumbnails"))
            })
            .and_then(Value::as_array)
            .and_then(|thumbs| thumbs.last())
            .and_then(|thumb| thumb.get("url"))
            .and_then(Value::as_str)
            .map(String::from);

        let mut top_songs = Vec::new();
        let mut albums = Vec::new();
        if let Some(sections) = find_key(&response, "sectionListRenderer")
            .and_then(|s| s.get("contents"))
            .and_then(Value::as_array)
        {
            for section in sections {
                if let Some(shelf) = section.get("musicShelfRenderer") {
                    if let Some(items) = shelf.get("contents").and_then(Value::as_array) {
                        for item in items {
                            if let Some(track) = parse_track_item(item) {
                                top_songs.push(track);
                            }
                        }
                    }
                } else if let Some(carousel) = section
                    .get("musicCarouselShelfRenderer")
                    .or_else(|| section.get("musicTwoRowItemRenderer").map(|_| section))
                {
                    if let Some(items) = carousel.get("contents").and_then(Value::as_array) {
                        for item in items {
                            if let Some(entry) = parse_shelf_item(item) {
                                // Skip "featured on" playlists and the
                                // "fans might also like" artist shelf.
                                if entry.id.starts_with("MPRE") || entry.id.starts_with("OLAK") {
                                    albums.push(entry);
                                }
                            }
                        }
                    }
                }
            }
        }
        top_songs.truncate(20);
        Ok(ArtistPage {
            name,
            thumbnail,
            top_songs,
            albums,
        })
    }

    /// Album page: header plus the tracklist shelf.
    pub async fn album_page(&self, browse_id: &str) -> Result<AlbumPage, String> {
        let response = self.browse(browse_id).await?;
        let header = find_key(&response, "musicDetailHeaderRenderer")
            .or_else(|| find_key(&response, "musicResponsiveHeaderRenderer"))
            .or_else(|| find_key(&response, "musicImmersiveHeaderRenderer"));
        let title = header
            .and_then(|h| h.pointer("/title/runs/0/text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // Subtitle runs are like ["Album", " • ", "EDEN", " • ", "2022"] —
        // skip separators, type labels, and bare years.
        const TYPE_LABELS: &[&str] = &[
            "Song", "Video", "Single", "Album", "EP", "Playlist", "Profile", "Podcast", "Episode",
        ];
        let artist = header
            .and_then(|h| h.pointer("/subtitle/runs"))
            .and_then(Value::as_array)
            .map(|runs| {
                runs.iter()
                    .filter_map(|run| run.get("text").and_then(Value::as_str))
                    .find(|text| {
                        text.trim() != "•"
                            && !TYPE_LABELS.contains(text)
                            && !text.chars().all(|c| c.is_ascii_digit())
                    })
                    .unwrap_or_default()
                    .to_string()
            })
            .filter(|a| !a.is_empty())
            .or_else(|| {
                // Newer headers carry no subtitle; the microformat describes
                // the page as "Album • <artist>".
                response
                    .pointer("/microformat/microformatDataRenderer/description")
                    .and_then(Value::as_str)
                    .and_then(|desc| desc.rsplit(" • ").next().map(String::from))
            })
            .unwrap_or_default();
        let thumbnail = header
            .and_then(|h| {
                h.pointer("/thumbnail/croppedSquareThumbnailRenderer/thumbnail/thumbnails")
                    .or_else(|| h.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"))
            })
            .and_then(Value::as_array)
            .and_then(|thumbs| thumbs.last())
            .and_then(|thumb| thumb.get("url"))
            .and_then(Value::as_str)
            .map(String::from);

        let mut tracks = Vec::new();
        if let Some(sections) = find_key(&response, "sectionListRenderer")
            .and_then(|s| s.get("contents"))
            .and_then(Value::as_array)
        {
            for section in sections {
                if let Some(items) = section
                    .get("musicShelfRenderer")
                    .and_then(|shelf| shelf.get("contents"))
                    .and_then(Value::as_array)
                {
                    for item in items {
                        if let Some(track) = parse_track_item(item) {
                            tracks.push(track);
                        }
                    }
                }
            }
        }
        // Album tracklist items omit the artist column and per-row
        // thumbnails — backfill both from the album header.
        for track in &mut tracks {
            if track.artist.is_empty() {
                track.artist = artist.clone();
            }
            if track.thumbnail.is_none() {
                track.thumbnail = thumbnail.clone();
            }
        }
        Ok(AlbumPage {
            title,
            artist,
            thumbnail,
            tracks,
        })
    }

    /// Resolves and downloads the best direct audio stream for a track.
    pub async fn download_audio(&self, video_id: &str, output_path: &Path) -> Result<(), String> {
        let response: Value = self
            .http
            .post(format!("{PLAYER_URL}?key={ANDROID_API_KEY}"))
            .header(
                reqwest::header::USER_AGENT,
                "com.google.android.youtube/20.10.38 (Linux; U; Android 11) gzip",
            )
            .json(&json!({
                "context": {
                    "client": {
                        "clientName": "ANDROID",
                        "clientVersion": "20.10.38",
                        "androidSdkVersion": 30,
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "videoId": video_id
            }))
            .send()
            .await
            .map_err(|e| format!("YouTube audio lookup failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("invalid YouTube audio response: {e}"))?;

        if response
            .pointer("/playabilityStatus/status")
            .and_then(Value::as_str)
            != Some("OK")
        {
            let reason = response
                .pointer("/playabilityStatus/reason")
                .and_then(Value::as_str)
                .unwrap_or("track unavailable");
            return Err(format!("YouTube audio unavailable: {reason}"));
        }

        let formats = response
            .pointer("/streamingData/formats")
            .and_then(Value::as_array)
            .ok_or_else(|| "YouTube returned no compatible formats".to_string())?;
        let stream_url = formats
            .iter()
            .find(|format| format.get("itag").and_then(Value::as_u64) == Some(18))
            .or_else(|| {
                formats.iter().find(|format| {
                    format
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .is_some_and(|mime| mime.starts_with("video/mp4"))
                })
            })
            .and_then(|format| format.get("url"))
            .and_then(Value::as_str)
            .ok_or_else(|| "YouTube returned no compatible MP4 audio URL".to_string())?;

        let response = self
            .http
            .get(stream_url)
            .send()
            .await
            .map_err(|e| format!("audio download failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "audio download returned HTTP {}",
                response.status()
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("invalid audio stream: {e}"))?;
        if bytes.len() < 12 || &bytes[4..8] != b"ftyp" || &bytes[8..12] == b"dash" {
            return Err("YouTube returned an unsupported audio container".to_string());
        }
        tokio::fs::write(output_path, bytes)
            .await
            .map_err(|e| format!("failed to save audio: {e}"))
    }

    pub async fn lyrics(
        &self,
        video_id: &str,
        title: &str,
        artist: &str,
        duration_ms: u64,
    ) -> Result<Option<LyricsResult>, String> {
        if let Ok(Some(lyrics)) = self.youtube_music_lyrics(video_id).await {
            return Ok(Some(lyrics));
        }

        let variants = lyric_variants(title, artist);
        for (clean_title, clean_artist) in &variants {
            if let Some(lyrics) = self.lrclib(clean_title, clean_artist, duration_ms).await? {
                return Ok(Some(lyrics));
            }
            if let Some(lyrics) = self
                .better_lyrics(clean_title, clean_artist, duration_ms)
                .await
            {
                return Ok(Some(lyrics));
            }
            if let Some(lyrics) = self.lyrics_ovh(clean_title, clean_artist).await {
                return Ok(Some(lyrics));
            }
        }
        for (clean_title, clean_artist) in &variants {
            if let Some(lyrics) = self.genius(clean_title, clean_artist).await {
                return Ok(Some(lyrics));
            }
        }
        Ok(None)
    }

    /// Last-resort lyrics via Genius's search endpoint and page extraction.
    /// Returns plain lyrics only; Genius does not provide timestamps.
    async fn genius(&self, title: &str, artist: &str) -> Option<LyricsResult> {
        const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
        let mut page_url = None;
        for term in [format!("{title} {artist}"), title.to_string()] {
            let mut url = url::Url::parse("https://genius.com/api/search/multi").ok()?;
            url.query_pairs_mut().append_pair("q", &term);
            let response: Value = self
                .http
                .get(url)
                .header(reqwest::header::USER_AGENT, BROWSER_UA)
                .send()
                .await
                .ok()?
                .json()
                .await
                .ok()?;
            page_url = response
                .pointer("/response/sections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|section| section.get("type").and_then(Value::as_str) == Some("song"))
                .flat_map(|section| {
                    section
                        .pointer("/hits")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                })
                .find_map(|hit| {
                    hit.pointer("/result/url")
                        .and_then(Value::as_str)
                        .map(String::from)
                });
            if page_url.is_some() {
                break;
            }
        }
        let page_url = page_url?;
        let html = self
            .http
            .get(&page_url)
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        extract_genius_lyrics(&html)
    }

    async fn youtube_music_lyrics(&self, video_id: &str) -> Result<Option<LyricsResult>, String> {
        let body = json!({
            "context": Self::web_remix_context(),
            "enablePersistentPlaylistPanel": true,
            "isAudioOnly": true,
            "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
            "videoId": video_id,
            "playlistId": format!("RDAMVM{video_id}"),
            "watchEndpointMusicSupportedConfigs": {
                "watchEndpointMusicConfig": {
                    "hasPersistentPlaylistPanel": true,
                    "musicVideoType": "MUSIC_VIDEO_TYPE_ATV"
                }
            }
        });
        let response: Value = self
            .http
            .post(format!(
                "https://music.youtube.com/youtubei/v1/next?key={INNERTUBE_API_KEY}"
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("YouTube Music lyrics lookup failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("invalid YouTube Music lyrics lookup: {e}"))?;
        let Some(browse_id) = find_browse_id(&response, "MUSIC_PAGE_TYPE_TRACK_LYRICS") else {
            return Ok(None);
        };

        let response: Value = self
            .http
            .post(format!(
                "https://music.youtube.com/youtubei/v1/browse?key={INNERTUBE_API_KEY}"
            ))
            .json(&json!({
                "context": {
                    "client": {
                        "clientName": "ANDROID_MUSIC",
                        "clientVersion": "7.21.50",
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "browseId": browse_id
            }))
            .send()
            .await
            .map_err(|e| format!("YouTube Music lyrics request failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("invalid YouTube Music lyrics response: {e}"))?;

        if let Some(timed) = find_key(&response, "timedLyricsData").and_then(Value::as_array) {
            let lines = timed
                .iter()
                .filter_map(|line| {
                    let start = line
                        .pointer("/cueRange/startTimeMilliseconds")
                        .and_then(Value::as_str)
                        .and_then(|time| time.parse::<u64>().ok())?;
                    let text = line.get("lyricLine").and_then(Value::as_str)?;
                    Some(format_lrc_line(start, text))
                })
                .collect::<Vec<_>>();
            if !lines.is_empty() {
                return Ok(Some(LyricsResult {
                    synced_lyrics: Some(lines.join("\n")),
                    plain_lyrics: None,
                }));
            }
        }

        if let Some(description) = find_key(&response, "musicDescriptionShelfRenderer") {
            let lyrics = description
                .pointer("/description/runs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect::<String>();
            if !lyrics.trim().is_empty() {
                return Ok(Some(LyricsResult {
                    synced_lyrics: None,
                    plain_lyrics: Some(lyrics),
                }));
            }
        }
        Ok(None)
    }

    async fn lrclib(
        &self,
        title: &str,
        artist: &str,
        duration_ms: u64,
    ) -> Result<Option<LyricsResult>, String> {
        let duration = (duration_ms / 1000).to_string();
        for attempt in 0..3 {
            let response = self
                .http
                .get("https://lrclib.net/api/get")
                .header(reqwest::header::USER_AGENT, "DeskTube/0.1.0")
                .query(&[
                    ("track_name", title),
                    ("artist_name", artist),
                    ("duration", duration.as_str()),
                ])
                .send()
                .await
                .map_err(|e| format!("lyrics request failed: {e}"))?;

            let status = response.status();
            if status.is_success() {
                let lyrics = response
                    .json::<LyricsResult>()
                    .await
                    .map_err(|e| format!("invalid lyrics response: {e}"))?;
                if lyrics.synced_lyrics.is_some() || lyrics.plain_lyrics.is_some() {
                    return Ok(Some(lyrics));
                }
            }
            if status == reqwest::StatusCode::NOT_FOUND {
                break;
            }
            if status.is_server_error() && attempt < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt + 1))).await;
                continue;
            }
            break;
        }

        let response = self
            .http
            .get("https://lrclib.net/api/search")
            .header(reqwest::header::USER_AGENT, "DeskTube/0.1.0")
            .query(&[("track_name", title), ("artist_name", artist)])
            .send()
            .await;
        let Ok(response) = response else {
            return Ok(None);
        };
        if !response.status().is_success() {
            return Ok(None);
        }

        let target_duration = duration_ms as f64 / 1000.0;
        let best = response
            .json::<Vec<LyricsSearchResult>>()
            .await
            .map_err(|e| format!("invalid lyrics search response: {e}"))?
            .into_iter()
            .filter(|lyrics| lyrics.synced_lyrics.is_some() || lyrics.plain_lyrics.is_some())
            .min_by(|left, right| {
                (left.duration - target_duration)
                    .abs()
                    .total_cmp(&(right.duration - target_duration).abs())
            });

        Ok(best.map(|lyrics| LyricsResult {
            synced_lyrics: lyrics.synced_lyrics,
            plain_lyrics: lyrics.plain_lyrics,
        }))
    }

    async fn better_lyrics(
        &self,
        title: &str,
        artist: &str,
        duration_ms: u64,
    ) -> Option<LyricsResult> {
        let duration = (duration_ms / 1000).to_string();
        let response = self
            .http
            .get("https://api.betterlyrics.org/getLyrics")
            .query(&[("s", title), ("a", artist), ("d", duration.as_str())])
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let ttml = response
            .json::<Value>()
            .await
            .ok()?
            .get("ttml")
            .and_then(Value::as_str)?
            .to_string();
        ttml_to_lrc(&ttml).map(|synced| LyricsResult {
            synced_lyrics: Some(synced),
            plain_lyrics: None,
        })
    }

    async fn lyrics_ovh(&self, title: &str, artist: &str) -> Option<LyricsResult> {
        let lyrics = lyrics_ovh_lookup(&self.http, artist, title).await;
        if lyrics.is_some() {
            return lyrics;
        }

        let mut suggestion_url = url::Url::parse("https://api.lyrics.ovh/suggest").ok()?;
        suggestion_url
            .path_segments_mut()
            .ok()?
            .push(&format!("{artist} {title}"));
        let response = self.http.get(suggestion_url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let suggestions = response.json::<Value>().await.ok()?;
        for item in suggestions
            .get("data")
            .and_then(Value::as_array)?
            .iter()
            .take(3)
        {
            let artist = item.pointer("/artist/name").and_then(Value::as_str)?;
            let title = item.get("title").and_then(Value::as_str)?;
            if let Some(lyrics) = lyrics_ovh_lookup(&self.http, artist, title).await {
                return Some(lyrics);
            }
        }
        None
    }
}

fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|child| find_key(child, key))
        }
        Value::Array(items) => items.iter().find_map(|child| find_key(child, key)),
        _ => None,
    }
}

fn find_browse_id(value: &Value, page_type: &str) -> Option<String> {
    if let Some(browse_id) = value
        .pointer("/browseEndpoint/browseId")
        .and_then(Value::as_str)
    {
        let kind = value
            .pointer("/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
            .and_then(Value::as_str);
        if kind == Some(page_type)
            || (page_type == "MUSIC_PAGE_TYPE_TRACK_LYRICS" && browse_id.starts_with("MPLY"))
        {
            return Some(browse_id.to_string());
        }
    }
    match value {
        Value::Object(map) => map
            .values()
            .find_map(|child| find_browse_id(child, page_type)),
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_browse_id(child, page_type)),
        _ => None,
    }
}

fn format_lrc_line(start_ms: u64, text: &str) -> String {
    let minutes = start_ms / 60_000;
    let seconds = (start_ms % 60_000) as f64 / 1000.0;
    format!("[{minutes:02}:{seconds:05.2}]{text}")
}

fn ttml_to_lrc(ttml: &str) -> Option<String> {
    let document = roxmltree::Document::parse(ttml).ok()?;
    let lines = document
        .descendants()
        .filter(|node| node.has_tag_name("p"))
        .filter_map(|node| {
            let start = node.attribute("begin").and_then(parse_ttml_time)?;
            let text = node
                .descendants()
                .filter(|child| child.is_text())
                .filter_map(|child| child.text())
                .collect::<String>()
                .trim()
                .to_string();
            (!text.is_empty()).then(|| format_lrc_line(start, &text))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn parse_ttml_time(value: &str) -> Option<u64> {
    if let Some(ms) = value.strip_suffix("ms") {
        return ms.parse::<f64>().ok().map(|value| value as u64);
    }
    if let Some(seconds) = value.strip_suffix('s') {
        return seconds
            .parse::<f64>()
            .ok()
            .map(|value| (value * 1000.0) as u64);
    }
    let parts = value.split(':').collect::<Vec<_>>();
    let seconds = match parts.as_slice() {
        [minutes, seconds] => minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?,
        [hours, minutes, seconds] => {
            hours.parse::<f64>().ok()? * 3600.0
                + minutes.parse::<f64>().ok()? * 60.0
                + seconds.parse::<f64>().ok()?
        }
        _ => return None,
    };
    Some((seconds * 1000.0) as u64)
}

fn lyric_variants(title: &str, artist: &str) -> Vec<(String, String)> {
    let cleaned_artist = clean_artist(artist);
    let cleaned_title = clean_title(title, &cleaned_artist);
    let mut variants = vec![
        (cleaned_title, cleaned_artist.clone()),
        (title.trim().to_string(), cleaned_artist),
        (title.trim().to_string(), artist.trim().to_string()),
    ];
    variants.dedup();
    variants
        .into_iter()
        .filter(|(title, _)| !title.is_empty())
        .collect()
}

fn clean_artist(artist: &str) -> String {
    let mut artist = artist.trim().to_string();
    for suffix in [" - topic", " vevo"] {
        if artist.to_ascii_lowercase().ends_with(suffix) {
            artist.truncate(artist.len() - suffix.len());
        }
    }
    artist
        .split([',', '&'])
        .next()
        .unwrap_or(artist.as_str())
        .trim()
        .to_string()
}

fn clean_title(title: &str, artist: &str) -> String {
    let mut title = title.trim().to_string();
    let artist = artist.trim();
    if !artist.is_empty() {
        let prefix = format!("{artist} - ").to_ascii_lowercase();
        if title.to_ascii_lowercase().starts_with(&prefix) {
            title = title[prefix.len()..].to_string();
        }
    }

    let mut cleaned = String::new();
    let mut depth = 0_usize;
    let mut bracket = String::new();
    for character in title.chars() {
        match character {
            '(' | '[' => {
                depth += 1;
                bracket.clear();
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                let removable = [
                    "official",
                    "audio",
                    "video",
                    "lyric",
                    "visualizer",
                    "hd",
                    "remaster",
                ]
                .iter()
                .any(|word| bracket.to_ascii_lowercase().contains(word));
                if !removable {
                    cleaned.push(' ');
                    cleaned.push_str(&bracket);
                    cleaned.push(' ');
                }
            }
            _ if depth > 0 => bracket.push(character),
            _ => cleaned.push(character),
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn lyrics_ovh_lookup(
    http: &reqwest::Client,
    artist: &str,
    title: &str,
) -> Option<LyricsResult> {
    let mut url = url::Url::parse("https://api.lyrics.ovh/v1/").ok()?;
    url.path_segments_mut().ok()?.extend([artist, title]);
    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let lyrics = response
        .json::<Value>()
        .await
        .ok()?
        .get("lyrics")
        .and_then(Value::as_str)?
        .replace("\r\n", "\n");
    (!lyrics.trim().is_empty()).then_some(LyricsResult {
        synced_lyrics: None,
        plain_lyrics: Some(lyrics),
    })
}

/// Ranks a result against the query. Title token hits outweigh artist hits,
/// but a query shaped like "<artist> <title>" ranks the artist's own tracks
/// above covers/uploads whose titles merely contain the artist's name.
/// Returns 0 for unrelated items so junk sinks to the bottom.
fn relevance_score(track: &TrackResult, tokens: &[String]) -> u32 {
    let title = normalize(&track.title);
    let artist = normalize(&track.artist);
    let query = normalize(&tokens.join(" "));
    let mut score = 0;
    for token in tokens {
        let token = token.to_lowercase();
        if title.split_whitespace().any(|word| word == token) {
            score += 4;
        } else if title.contains(&token) {
            score += 3;
        } else if artist.split_whitespace().any(|word| word == token) {
            score += 2;
        } else if artist.contains(&token) {
            score += 1;
        }
    }
    let combined = format!("{artist} {title}");
    if artist == query {
        score += 40;
    } else if combined == query {
        score += 30;
    } else if title == query {
        score += 20;
    } else if combined.starts_with(&query) || title.starts_with(&query) {
        score += 10;
    } else if title.contains(&query) || combined.contains(&query) {
        score += 6;
    }
    score
}

/// Lowercases and collapses punctuation into single spaces so
/// "brakence - introvert" compares equal to "brakence introvert".
fn normalize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_search_results(resp: &Value) -> Vec<TrackResult> {
    let mut out = Vec::new();
    let shelves = resp.pointer(
        "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
    );
    let Some(shelves) = shelves.and_then(Value::as_array) else {
        return out;
    };

    for shelf in shelves {
        let items = shelf.pointer("/musicShelfRenderer/contents");
        let Some(items) = items.and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if let Some(t) = parse_track_item(item) {
                out.push(t);
            }
        }
    }
    out
}

/// Recursively finds the browseId of a typed page (artist, album) inside an
/// item — they live in the "Go to artist"/"Go to album" menu entries, not
/// the text runs.
fn find_page_browse_id(value: &Value, page_type: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(endpoint) = map.get("browseEndpoint") {
                let kind = endpoint
                    .pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                    .and_then(Value::as_str);
                if kind == Some(page_type) {
                    if let Some(id) = endpoint.get("browseId").and_then(Value::as_str) {
                        return Some(id.to_string());
                    }
                }
            }
            map.values()
                .find_map(|child| find_page_browse_id(child, page_type))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_page_browse_id(child, page_type)),
        _ => None,
    }
}

/// (text, browseId) pairs from a flex-column runs array.
fn col_runs(col: Option<&Value>) -> Vec<(String, Option<String>)> {
    col.and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| {
                    let text = run.get("text")?.as_str()?.to_string();
                    let browse_id = run
                        .pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(Value::as_str)
                        .map(String::from);
                    Some((text, browse_id))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_track_item(item: &Value) -> Option<TrackResult> {
    let r = item.get("musicResponsiveListItemRenderer")?;
    let video_id = r
        .pointer("/playlistItemData/videoId")
        .or_else(|| {
            r.pointer(
                "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
            )
        })
        .or_else(|| find_key(r, "watchEndpoint").and_then(|w| w.get("videoId")))?
        .as_str()?
        .to_string();

    let cols = r.pointer("/flexColumns")?.as_array()?;
    let col_text = |i: usize| -> Vec<String> {
        col_runs(cols.get(i))
            .into_iter()
            .map(|(text, _)| text)
            .collect()
    };

    let title = col_text(0).into_iter().next()?;
    let meta = col_text(1);
    // meta is typically ["Song", "•", "Artist", "•", "Album", "•", "3:45"] or
    // ["Video", "•", "Uploader", "•", "38K views", "•", "2:45"] — skip the
    // result-type label, separators, durations, and view counts.
    const TYPE_LABELS: &[&str] = &[
        "Song", "Video", "Single", "Album", "EP", "Playlist", "Profile", "Podcast", "Episode",
    ];
    let is_meta_value = |s: &String| {
        s.trim() != "•"
            && !s.contains(':')
            && !TYPE_LABELS.contains(&s.as_str())
            && !s.to_lowercase().contains("view")
    };
    let artist = meta
        .iter()
        .find(|s| is_meta_value(s))
        .cloned()
        .unwrap_or_default();
    let duration = meta
        .iter()
        .find(|s| s.contains(':'))
        .cloned()
        .or_else(|| {
            // Album pages put the duration in a later flex column.
            (1..cols.len()).flat_map(col_text).find(|s| s.contains(':'))
        })
        .unwrap_or_default();
    let duration_ms = parse_duration_ms(&duration);

    // Browse IDs for artist/album navigation live in the item's menu.
    let artist_id = find_page_browse_id(r, "MUSIC_PAGE_TYPE_ARTIST");
    let album_id = find_page_browse_id(r, "MUSIC_PAGE_TYPE_ALBUM");
    // meta values after the artist are usually [album].
    let album = meta
        .iter()
        .filter(|s| is_meta_value(s))
        .nth(1)
        .cloned()
        .filter(|_| album_id.is_some());

    let thumbnail = r
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|thumbnails| thumbnails.last())
        .and_then(|thumbnail| thumbnail.get("url"))
        .and_then(Value::as_str)
        .map(String::from);

    Some(TrackResult {
        video_id,
        title,
        artist,
        duration,
        duration_ms,
        thumbnail,
        artist_id,
        album_id,
        album,
    })
}

/// Parses a two-row card (albums/singles shelves) into a ShelfItem.
fn parse_shelf_item(item: &Value) -> Option<ShelfItem> {
    let r = item.get("musicTwoRowItemRenderer")?;
    let id = r
        .pointer("/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)?
        .to_string();
    let title = r
        .pointer("/title/runs/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let subtitle = r
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .filter(|text| *text != " • ")
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default();
    let thumbnail = r
        .pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .and_then(|thumbs| thumbs.last())
        .and_then(|thumb| thumb.get("url"))
        .and_then(Value::as_str)
        .map(String::from);
    Some(ShelfItem {
        id,
        title,
        subtitle,
        thumbnail,
    })
}

fn extract_genius_lyrics(html: &str) -> Option<LyricsResult> {
    const MARKER: &str = "data-lyrics-container=\"true\"";
    let mut out = String::new();
    let mut rest = html;
    while let Some(pos) = rest.find(MARKER) {
        let after = &rest[pos + MARKER.len()..];
        let Some(start) = after.find('>') else {
            break;
        };
        let body = &after[start + 1..];
        let Some(end) = body.find("</div>") else {
            break;
        };
        let chunk = html_to_text(&body[..end]);
        let chunk = strip_genius_header(&chunk);
        if !chunk.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(chunk);
        }
        rest = &body[end..];
    }
    (!out.trim().is_empty()).then_some(LyricsResult {
        synced_lyrics: None,
        plain_lyrics: Some(out.trim().to_string()),
    })
}

/// Drops the "N Contributors<Title> Lyrics" header Genius injects into the
/// first lyric container.
fn strip_genius_header(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(contributors) = trimmed.find("Contributors") else {
        return trimmed;
    };
    if contributors > 16 {
        return trimmed;
    }
    match trimmed.find("Lyrics") {
        Some(index) => trimmed[index + 6..].trim_start(),
        None => trimmed,
    }
}

fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find('<') {
        text.push_str(&rest[..pos]);
        let Some(end) = rest[pos..].find('>') else {
            break;
        };
        let tag = &rest[pos + 1..pos + end];
        if tag.trim_start_matches('/').starts_with("br") {
            text.push('\n');
        }
        rest = &rest[pos + end + 1..];
    }
    text.push_str(rest);
    decode_entities(&text)
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let Some(end) = rest[pos..].find(';') else {
            out.push_str(&rest[pos..]);
            break;
        };
        let entity = &rest[pos + 1..pos + end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "nbsp" => Some(' '),
            "apos" | "#39" | "#x27" => Some('\''),
            "#x2019" | "#8217" | "#x2018" | "#8216" => Some('’'),
            "#x201c" | "#8220" | "#x201d" | "#8221" => Some('"'),
            _ if entity.starts_with("#x") => u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        match decoded {
            Some(character) => out.push(character),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &rest[pos + end + 1..];
    }
    out.push_str(rest);
    out
}

fn parse_duration_ms(d: &str) -> u64 {
    let parts: Vec<u64> = d.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [m, s] => (m * 60 + s) * 1000,
        [h, m, s] => (h * 3600 + m * 60 + s) * 1000,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::Innertube;
    use std::io::Read;

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn search_probe() {
        let query = std::env::var("SEARCH_PROBE").unwrap_or_else(|_| "eden false alarm".into());
        let results = Innertube::new().search(&query).await.expect("search");
        assert!(!results.is_empty());
        for track in &results {
            eprintln!("{} - {}", track.artist, track.title);
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn browse_probe() {
        let it = Innertube::new();
        let results = it.search("brakence introvert").await.expect("search");
        for t in &results {
            eprintln!(
                "{} | {} | artist_id={:?} album_id={:?} album={:?}",
                t.artist, t.title, t.artist_id, t.album_id, t.album
            );
        }
        let artist_id = results
            .iter()
            .find_map(|t| t.artist_id.clone())
            .expect("artist id");
        eprintln!("artist_id {artist_id}");
        let page = it.artist_page(&artist_id).await.expect("artist page");
        eprintln!(
            "artist {} | {} songs | {} albums",
            page.name,
            page.top_songs.len(),
            page.albums.len()
        );
        for album in &page.albums {
            eprintln!("  album {} [{}]", album.title, album.id);
        }
        if let Some(album) = page.albums.first() {
            let page = it.album_page(&album.id).await.expect("album page");
            eprintln!(
                "album {} by {} | {} tracks | first={:?} thumb={}",
                page.title,
                page.artist,
                page.tracks.len(),
                page.tracks.first().map(|t| (&t.artist, &t.title)),
                page.tracks
                    .first()
                    .map(|t| t.thumbnail.is_some())
                    .unwrap_or(false)
            );
        }
        let radio = it.radio(&results[0].video_id).await.expect("radio");
        eprintln!("radio {} tracks", radio.len());
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn lyrics_probe() {
        let result = Innertube::new()
            .lyrics(
                "",
                "EDEN - Pathetic! / False Alarm",
                "mcmxcv songs",
                165_000,
            )
            .await
            .expect("lyrics call");
        eprintln!("{result:?}");
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn genius_probe() {
        const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
        let http = reqwest::Client::new();
        let search = http
            .get("https://genius.com/api/search/multi?q=EDEN+false+alarm")
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .send()
            .await;
        match search {
            Ok(r) => eprintln!("search status={}", r.status()),
            Err(e) => eprintln!("search err={e}"),
        }
        let page = http
            .get("https://genius.com/Eden-pathetic-false-alarm-lyrics")
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .send()
            .await;
        match page {
            Ok(r) => {
                eprintln!("page status={}", r.status());
                let html = r.text().await.unwrap_or_default();
                eprintln!(
                    "markers={} extracted={}",
                    html.matches("data-lyrics-container=\"true\"").count(),
                    super::extract_genius_lyrics(&html)
                        .map(|l| l.plain_lyrics.map(|t| t.len()).unwrap_or(0))
                        .unwrap_or(0)
                );
            }
            Err(e) => eprintln!("page err={e}"),
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn direct_audio_probe() {
        let path = std::env::temp_dir().join("desktube-audio-probe.mp4");
        Innertube::new()
            .download_audio("dQw4w9WgXcQ", &path)
            .await
            .expect("direct audio download");
        let mut header = [0_u8; 12];
        std::fs::File::open(path)
            .expect("downloaded audio")
            .read_exact(&mut header)
            .expect("header");
        assert_eq!(&header[4..8], b"ftyp");
        assert_ne!(&header[8..12], b"dash");
    }
}
