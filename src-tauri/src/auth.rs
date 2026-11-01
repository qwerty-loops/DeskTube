use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

const KEYRING_SERVICE: &str = "com.desktube.app";
const KEYRING_USER: &str = "google-oauth-token";
const YOUTUBE_API: &str = "https://www.googleapis.com/youtube/v3";
const YOUTUBE_MUSIC_API: &str = "https://music.youtube.com/youtubei/v1/browse";
const DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";
const DEVICE_GRANT_TYPE: &str = "http://oauth.net/grant_type/device/1.0";
const LIBRARY_PAGE_LIMIT: usize = 60;
const LIBRARY_ITEM_LIMIT: usize = 3_000;

#[derive(Clone, Deserialize)]
struct OAuthFile {
    installed: OAuthCredentials,
}

#[derive(Clone, Deserialize)]
struct OAuthCredentials {
    client_id: String,
    client_secret: String,
    token_uri: String,
    #[serde(default)]
    redirect_uris: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct TokenSet {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    client_id: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct DeviceAuthPrompt {
    pub verification_url: String,
    pub user_code: String,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Clone, Serialize)]
pub struct AccountProfile {
    pub name: String,
    pub avatar: Option<String>,
    pub channel_id: String,
}

#[derive(Clone, Serialize)]
pub struct AuthStatus {
    pub signed_in: bool,
    pub profile: Option<AccountProfile>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LibraryTrack {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration: String,
    pub duration_ms: u64,
    pub thumbnail: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thumbnail: Option<String>,
    pub item_count: u64,
}

pub struct AuthManager {
    http: reqwest::Client,
    credentials: OAuthCredentials,
    token: RwLock<Option<TokenSet>>,
    visitor_id: RwLock<Option<String>>,
}

impl AuthManager {
    pub fn new() -> Result<Arc<Self>, String> {
        let app_data =
            std::env::var("APPDATA").map_err(|_| "APPDATA is unavailable".to_string())?;
        let config_dir = std::path::Path::new(&app_data).join("DeskTube");
        let desktop_path = config_dir.join("google-oauth-desktop.json");
        let config_path = if desktop_path.exists() {
            desktop_path
        } else {
            config_dir.join("google-oauth.json")
        };
        let credentials = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|config| {
                serde_json::from_str::<OAuthFile>(&config)
                    .map(|file| file.installed)
                    .ok()
            })
            .unwrap_or_else(|| OAuthCredentials {
                client_id: String::new(),
                client_secret: String::new(),
                token_uri: "https://oauth2.googleapis.com/token".to_string(),
                redirect_uris: Vec::new(),
            });
        let token = load_token().and_then(|token| {
            if credentials.client_id.is_empty() {
                return None;
            }
            if token.client_id.as_deref() == Some(credentials.client_id.as_str()) {
                Some(token)
            } else {
                None
            }
        });
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("DeskTube/0.1.0")
            .build()
            .map_err(|e| format!("failed to create OAuth client: {e}"))?;
        Ok(Arc::new(Self {
            http,
            credentials,
            token: RwLock::new(token),
            visitor_id: RwLock::new(None),
        }))
    }

    pub async fn begin_sign_in(
        self: &Arc<Self>,
        app: AppHandle,
    ) -> Result<DeviceAuthPrompt, String> {
        if self.credentials.client_id.is_empty() || self.credentials.client_secret.is_empty() {
            return Err("Google sign-in is not configured. Place your OAuth client JSON at %APPDATA%\\DeskTube\\google-oauth.json and restart DeskTube. You can still search and play music without signing in.".to_string());
        }
        if self.credentials.redirect_uris.iter().any(|uri| {
            url::Url::parse(uri).is_ok_and(|url| {
                url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            })
        }) {
            return self.begin_desktop_sign_in(app).await;
        }
        let response = self
            .http
            .post(DEVICE_CODE_URL)
            .header("User-Agent", "DeskTube/0.1.0 Cobalt/Version")
            .form(&[
                ("client_id", self.credentials.client_id.as_str()),
                ("scope", "https://www.googleapis.com/auth/youtube"),
            ])
            .send()
            .await
            .map_err(|e| format!("Google device sign-in failed: {e}"))?;
        if !response.status().is_success() {
            return Err("Google rejected the sign-in request — try again later.".to_string());
        }
        let device = response
            .json::<DeviceCodeResponse>()
            .await
            .map_err(|e| format!("invalid Google device response: {e}"))?;
        let prompt = DeviceAuthPrompt {
            verification_url: device.verification_url,
            user_code: device.user_code,
        };

        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            match manager
                .finish_device_sign_in(device.device_code, device.interval, device.expires_in)
                .await
            {
                Ok(()) => {
                    let _ = app.emit("auth-updated", true);
                }
                Err(error) => {
                    let _ = app.emit("auth-error", error);
                }
            }
        });
        Ok(prompt)
    }

    async fn begin_desktop_sign_in(
        self: &Arc<Self>,
        app: AppHandle,
    ) -> Result<DeviceAuthPrompt, String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Could not start the local Google sign-in callback".to_string())?;
        let port = listener
            .local_addr()
            .map_err(|_| "Could not determine callback port".to_string())?
            .port();
        let redirect = format!("http://127.0.0.1:{port}/oauth/callback");
        let mut random = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let verifier = URL_SAFE_NO_PAD.encode(random);
        rand::rngs::OsRng.fill_bytes(&mut random);
        let state = URL_SAFE_NO_PAD.encode(random);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut authorization = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .map_err(|_| "Invalid authorization endpoint".to_string())?;
        authorization.query_pairs_mut().extend_pairs([
            ("client_id", self.credentials.client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", "https://www.googleapis.com/auth/youtube"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(300), async {
                loop {
                    let (mut stream, _) = listener.accept().await.map_err(|_| "Sign-in callback failed".to_string())?;
                    let mut request = vec![0u8; 8192];
                    let size = match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut request)).await {
                        Ok(Ok(size)) if size > 0 => size,
                        _ => continue,
                    };
                    let request = String::from_utf8_lossy(&request[..size]);
                    let Some(target) = request.lines().next().and_then(|line| line.strip_prefix("GET ")).and_then(|line| line.split_whitespace().next()) else { continue };
                    let Ok(callback) = url::Url::parse(&format!("http://127.0.0.1{target}")) else { continue };
                    let params: std::collections::HashMap<_, _> = callback.query_pairs().into_owned().collect();
                    if callback.path() != "/oauth/callback" || params.get("state") != Some(&state) {
                        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        continue;
                    }
                    let result = if params.contains_key("error") {
                        Err("Google sign-in was cancelled or denied".to_string())
                    } else if let Some(code) = params.get("code") {
                        manager.exchange_desktop_code(code, &redirect, &verifier).await
                    } else {
                        Err("Google returned no sign-in code".to_string())
                    };
                    let body = if result.is_ok() { "Signed in to DeskTube. You can close this tab." } else { "Sign-in did not complete. Return to DeskTube for details." };
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(response.as_bytes()).await;
                    return result;
                }
            }).await.unwrap_or_else(|_| Err("Google sign-in timed out. Click Connect Google to try again.".to_string()));
            match result {
                Ok(()) => {
                    let _ = app.emit("auth-updated", true);
                }
                Err(error) => {
                    let _ = app.emit("auth-error", error);
                }
            }
        });
        Ok(DeviceAuthPrompt {
            verification_url: authorization.to_string(),
            user_code: String::new(),
        })
    }

    async fn exchange_desktop_code(
        &self,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> Result<(), String> {
        let response = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .timeout(Duration::from_secs(30))
            .form(&[
                ("client_id", self.credentials.client_id.as_str()),
                ("client_secret", self.credentials.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", redirect),
                ("code_verifier", verifier),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .map_err(|_| {
                "Google token exchange could not connect. Try signing in again.".to_string()
            })?;
        if !response.status().is_success() {
            return Err("Google rejected the sign-in exchange. Check the Desktop client configuration and test-user access.".to_string());
        }
        let response = response
            .json::<TokenResponse>()
            .await
            .map_err(|_| "Google returned an invalid token response".to_string())?;
        let token = TokenSet {
            access_token: response.access_token,
            refresh_token: response.refresh_token.ok_or_else(|| {
                "Google did not grant offline access. Try signing in again.".to_string()
            })?,
            expires_at: unix_time() + response.expires_in,
            client_id: Some(self.credentials.client_id.clone()),
        };
        save_token(&token)?;
        *self.token.write().await = Some(token);
        Ok(())
    }

    async fn finish_device_sign_in(
        &self,
        device_code: String,
        interval: u64,
        expires_in: u64,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(expires_in.min(900));
        let mut interval = interval.max(3);
        loop {
            if std::time::Instant::now() >= deadline {
                return Err("Google sign-in timed out".to_string());
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
            let response = self
                .http
                .post(&self.credentials.token_uri)
                .form(&[
                    ("client_id", self.credentials.client_id.as_str()),
                    ("client_secret", self.credentials.client_secret.as_str()),
                    ("code", device_code.as_str()),
                    ("grant_type", DEVICE_GRANT_TYPE),
                ])
                .send()
                .await
                .map_err(|e| format!("Google token exchange failed: {e}"))?;
            if response.status().is_success() {
                let response = response
                    .json::<TokenResponse>()
                    .await
                    .map_err(|e| format!("invalid token response: {e}"))?;
                let token = TokenSet {
                    access_token: response.access_token,
                    refresh_token: response
                        .refresh_token
                        .ok_or_else(|| "Google did not return a refresh token".to_string())?,
                    expires_at: unix_time() + response.expires_in,
                    client_id: Some(self.credentials.client_id.clone()),
                };
                save_token(&token)?;
                *self.token.write().await = Some(token);
                return Ok(());
            }

            let error = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|value| value.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or_default();
            match error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += 5;
                    continue;
                }
                "expired_token" => return Err("Google sign-in code expired".to_string()),
                "access_denied" => return Err("Google sign-in was denied".to_string()),
                _ => return Err(format!("Google sign-in failed: {error}")),
            }
        }
    }

    pub async fn sign_out(&self) -> Result<(), String> {
        *self.token.write().await = None;
        delete_token()
    }

    pub async fn status(&self) -> Result<AuthStatus, String> {
        if self.token.read().await.is_none() {
            return Ok(AuthStatus {
                signed_in: false,
                profile: None,
            });
        }
        let profile = self.profile().await?;
        Ok(AuthStatus {
            signed_in: true,
            profile: Some(profile),
        })
    }

    pub async fn profile(&self) -> Result<AccountProfile, String> {
        let value = self
            .youtube_get("channels", &[("part", "snippet"), ("mine", "true")])
            .await?;
        let channel = value
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| "Google account has no YouTube channel".to_string())?;
        Ok(AccountProfile {
            name: text_at(channel, "/snippet/title"),
            avatar: thumbnail_at(channel, "/snippet/thumbnails"),
            channel_id: text_at(channel, "/id"),
        })
    }

    pub async fn liked_songs(&self) -> Result<Vec<LibraryTrack>, String> {
        match self.youtube_music_playlist_tracks("LM").await {
            Ok(tracks) if !tracks.is_empty() => Ok(tracks),
            Ok(_) | Err(_) => match self.playlist_items_data_api("LM").await {
                Ok(tracks) => Ok(tracks),
                Err(_) => self.liked_videos_data_api().await,
            },
        }
    }

    pub async fn playlists(&self) -> Result<Vec<PlaylistSummary>, String> {
        let music_playlists = self.youtube_music_library_playlists().await;
        let data_playlists = self.playlists_data_api().await;
        match (music_playlists, data_playlists) {
            (Ok(music), Ok(data)) => Ok(merge_playlists(music, data)),
            (Ok(music), Err(_)) => Ok(music),
            (Err(_), Ok(data)) => Ok(data),
            (Err(music_error), Err(data_error)) => {
                Err(format!("{music_error}; YouTube fallback: {data_error}"))
            }
        }
    }

    pub async fn playlist_items(&self, playlist_id: &str) -> Result<Vec<LibraryTrack>, String> {
        match self.youtube_music_playlist_tracks(playlist_id).await {
            Ok(tracks) if !tracks.is_empty() => Ok(tracks),
            Ok(_) | Err(_) => self.playlist_items_data_api(playlist_id).await,
        }
    }

    async fn youtube_music_library_playlists(&self) -> Result<Vec<PlaylistSummary>, String> {
        let mut playlists = Vec::new();
        let mut continuation = None;
        for _ in 0..LIBRARY_PAGE_LIMIT {
            let response = self
                .youtube_music_browse("FEmusic_liked_playlists", continuation.as_deref())
                .await?;
            let grid = find_key(&response, "gridRenderer")
                .or_else(|| find_key(&response, "gridContinuation"))
                .ok_or_else(|| {
                    "YouTube Music returned an unexpected playlists response".to_string()
                })?;
            let items = grid.get("items").and_then(Value::as_array).ok_or_else(|| {
                "YouTube Music returned an unexpected playlists response".to_string()
            })?;
            playlists.extend(items.iter().filter_map(playlist_from_music_item));
            continuation = continuation_token(grid);
            if continuation.is_none() || playlists.len() >= LIBRARY_ITEM_LIMIT {
                break;
            }
        }
        playlists.truncate(LIBRARY_ITEM_LIMIT);
        Ok(playlists)
    }

    async fn youtube_music_playlist_tracks(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<LibraryTrack>, String> {
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{playlist_id}")
        };
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        let mut continuation = None;
        for _ in 0..LIBRARY_PAGE_LIMIT {
            let response = self
                .youtube_music_browse(&browse_id, continuation.as_deref())
                .await?;
            let shelf = find_key(&response, "musicPlaylistShelfRenderer")
                .or_else(|| find_key(&response, "musicPlaylistShelfContinuation"))
                .ok_or_else(|| {
                    "YouTube Music returned an unexpected playlist response".to_string()
                })?;
            let items = shelf
                .get("contents")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    "YouTube Music returned an unexpected playlist response".to_string()
                })?;
            for track in items.iter().filter_map(track_from_music_item) {
                if seen.insert(track.video_id.clone()) {
                    tracks.push(track);
                }
            }
            continuation = continuation_token(shelf);
            if continuation.is_none() || tracks.len() >= LIBRARY_ITEM_LIMIT {
                break;
            }
        }
        tracks.truncate(LIBRARY_ITEM_LIMIT);
        Ok(tracks)
    }

    async fn visitor_id(&self) -> Result<String, String> {
        if let Some(visitor_id) = self.visitor_id.read().await.as_ref() {
            return Ok(visitor_id.clone());
        }
        let page = self
            .http
            .get("https://music.youtube.com")
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:88.0) Gecko/20100101 Firefox/88.0",
            )
            .header(reqwest::header::ACCEPT, "text/html")
            .send()
            .await
            .map_err(|e| format!("YouTube Music session request failed: {e}"))?
            .text()
            .await
            .map_err(|e| format!("invalid YouTube Music session response: {e}"))?;
        let marker = "\"VISITOR_DATA\":\"";
        let start = page
            .find(marker)
            .ok_or_else(|| "YouTube Music did not provide a visitor session".to_string())?
            + marker.len();
        let end = page[start..]
            .find('"')
            .map(|index| start + index)
            .ok_or_else(|| "YouTube Music returned an invalid visitor session".to_string())?;
        let visitor_id = page[start..end].to_string();
        *self.visitor_id.write().await = Some(visitor_id.clone());
        Ok(visitor_id)
    }

    async fn youtube_music_browse(
        &self,
        browse_id: &str,
        continuation: Option<&str>,
    ) -> Result<Value, String> {
        let token = self.access_token().await?;
        let visitor_id = self.visitor_id().await?;
        let mut request = self
            .http
            .post(format!("{YOUTUBE_MUSIC_API}?alt=json"))
            .bearer_auth(token)
            .header("X-Goog-Visitor-Id", visitor_id)
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:88.0) Gecko/20100101 Firefox/88.0",
            )
            .header(reqwest::header::ACCEPT, "*/*")
            .header(reqwest::header::COOKIE, "SOCS=CAI")
            .header(reqwest::header::ORIGIN, "https://music.youtube.com")
            .header(reqwest::header::REFERER, "https://music.youtube.com/")
            .header("X-Goog-Request-Time", unix_time().to_string())
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", "1.20261029.01.00");
        if let Some(token) = continuation {
            request = request.query(&[("ctoken", token), ("continuation", token)]);
        }
        let response = request
            .json(&json!({
                "context": {
                    "client": {
                        "clientName": "WEB_REMIX",
                        "clientVersion": "1.20261029.01.00",
                        "hl": "en",
                        "gl": "US"
                    },
                    "user": {}
                },
                "browseId": browse_id
            }))
            .send()
            .await
            .map_err(|e| format!("YouTube Music library request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "YouTube Music library returned HTTP {status}: {}",
                detail.chars().take(300).collect::<String>()
            ));
        }
        response
            .json()
            .await
            .map_err(|e| format!("invalid YouTube Music library response: {e}"))
    }

    async fn liked_videos_data_api(&self) -> Result<Vec<LibraryTrack>, String> {
        let mut tracks = Vec::new();
        let mut page_token = String::new();
        for _ in 0..LIBRARY_PAGE_LIMIT {
            let mut query = vec![
                ("part", "snippet,contentDetails"),
                ("myRating", "like"),
                ("maxResults", "50"),
            ];
            if !page_token.is_empty() {
                query.push(("pageToken", page_token.as_str()));
            }
            let value = self.youtube_get("videos", &query).await?;
            tracks.extend(
                value
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|item| text_at(item, "/snippet/categoryId") == "10")
                    .map(track_from_video),
            );
            let Some(next) = value.get("nextPageToken").and_then(Value::as_str) else {
                break;
            };
            page_token = next.to_string();
        }
        Ok(tracks)
    }

    async fn playlists_data_api(&self) -> Result<Vec<PlaylistSummary>, String> {
        let mut playlists = Vec::new();
        let mut page_token = String::new();
        for _ in 0..LIBRARY_PAGE_LIMIT {
            let mut query = vec![
                ("part", "snippet,contentDetails"),
                ("mine", "true"),
                ("maxResults", "50"),
            ];
            if !page_token.is_empty() {
                query.push(("pageToken", page_token.as_str()));
            }
            let value = self.youtube_get("playlists", &query).await?;
            playlists.extend(
                value
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|item| PlaylistSummary {
                        id: text_at(item, "/id"),
                        title: text_at(item, "/snippet/title"),
                        description: text_at(item, "/snippet/description"),
                        thumbnail: thumbnail_at(item, "/snippet/thumbnails"),
                        item_count: item
                            .pointer("/contentDetails/itemCount")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    }),
            );
            let Some(next) = value.get("nextPageToken").and_then(Value::as_str) else {
                break;
            };
            page_token = next.to_string();
        }
        Ok(playlists)
    }

    async fn playlist_items_data_api(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<LibraryTrack>, String> {
        let mut ids = Vec::new();
        let mut page_token = String::new();
        for _ in 0..LIBRARY_PAGE_LIMIT {
            let mut query = vec![
                ("part", "contentDetails"),
                ("playlistId", playlist_id),
                ("maxResults", "50"),
            ];
            if !page_token.is_empty() {
                query.push(("pageToken", page_token.as_str()));
            }
            let value = self.youtube_get("playlistItems", &query).await?;
            ids.extend(
                value
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| {
                        item.pointer("/contentDetails/videoId")
                            .and_then(Value::as_str)
                            .map(String::from)
                    }),
            );
            let Some(next) = value.get("nextPageToken").and_then(Value::as_str) else {
                break;
            };
            page_token = next.to_string();
        }

        let mut tracks = Vec::new();
        for chunk in ids.chunks(50) {
            let id_list = chunk.join(",");
            let videos = self
                .youtube_get(
                    "videos",
                    &[
                        ("part", "snippet,contentDetails"),
                        ("id", id_list.as_str()),
                        ("maxResults", "50"),
                    ],
                )
                .await?;
            tracks.extend(
                videos
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(track_from_video),
            );
        }
        Ok(tracks)
    }

    pub async fn create_playlist(&self, title: &str) -> Result<PlaylistSummary, String> {
        let token = self.access_token().await?;
        let response = self
            .http
            .post(format!("{YOUTUBE_API}/playlists"))
            .query(&[("part", "snippet,status")])
            .bearer_auth(token)
            .json(&json!({
                "snippet": { "title": title, "description": "Created with DeskTube" },
                "status": { "privacyStatus": "private" }
            }))
            .send()
            .await
            .map_err(|e| format!("failed to create playlist: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("YouTube returned HTTP {}", response.status()));
        }
        let item = response.json::<Value>().await.map_err(|e| e.to_string())?;
        Ok(PlaylistSummary {
            id: text_at(&item, "/id"),
            title: text_at(&item, "/snippet/title"),
            description: text_at(&item, "/snippet/description"),
            thumbnail: thumbnail_at(&item, "/snippet/thumbnails"),
            item_count: 0,
        })
    }

    /// Rates a video through the YouTube Data API. `rating` is "like" or
    /// "none". YouTube liked videos feed YouTube Music recommendations; the
    /// private LM liked-songs list is not writable through a public API.
    pub async fn rate_track(&self, video_id: &str, rating: &str) -> Result<(), String> {
        let token = self.access_token().await?;
        let response = self
            .http
            .post(format!("{YOUTUBE_API}/videos/rate"))
            .query(&[("id", video_id), ("rating", rating)])
            .bearer_auth(token)
            .json(&json!({}))
            .send()
            .await
            .map_err(|e| format!("YouTube rating failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("YouTube returned HTTP {}", response.status()));
        }
        Ok(())
    }

    async fn youtube_get(&self, endpoint: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let token = self.access_token().await?;
        let response = self
            .http
            .get(format!("{YOUTUBE_API}/{endpoint}"))
            .query(query)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("YouTube request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("YouTube returned HTTP {}", response.status()));
        }
        response
            .json()
            .await
            .map_err(|e| format!("invalid YouTube response: {e}"))
    }

    async fn access_token(&self) -> Result<String, String> {
        {
            let token = self.token.read().await;
            let token = token
                .as_ref()
                .ok_or_else(|| "Sign in with Google first".to_string())?;
            if token.expires_at > unix_time() + 60 {
                return Ok(token.access_token.clone());
            }
        }

        let current = self
            .token
            .read()
            .await
            .clone()
            .ok_or_else(|| "Sign in with Google first".to_string())?;
        let response = self
            .http
            .post(&self.credentials.token_uri)
            .form(&[
                ("client_id", self.credentials.client_id.as_str()),
                ("client_secret", self.credentials.client_secret.as_str()),
                ("refresh_token", current.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| format!("token refresh failed: {e}"))?;
        if !response.status().is_success() {
            return Err("Google session expired. Please sign in again.".to_string());
        }
        let response = response
            .json::<TokenResponse>()
            .await
            .map_err(|e| e.to_string())?;
        let token = TokenSet {
            access_token: response.access_token,
            refresh_token: current.refresh_token,
            expires_at: unix_time() + response.expires_in,
            client_id: Some(self.credentials.client_id.clone()),
        };
        save_token(&token)?;
        *self.token.write().await = Some(token.clone());
        Ok(token.access_token)
    }
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_token() -> Option<TokenSet> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    let value = entry.get_password().ok()?;
    serde_json::from_str(&value).ok()
}

fn save_token(token: &TokenSet) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("credential store unavailable: {e}"))?;
    let value = serde_json::to_string(token).map_err(|e| e.to_string())?;
    entry
        .set_password(&value)
        .map_err(|e| format!("failed to store Google token: {e}"))
}

fn delete_token() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("credential store unavailable: {e}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("failed to delete stored token: {e}")),
    }
}

fn text_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn thumbnail_at(value: &Value, pointer: &str) -> Option<String> {
    let thumbnails = value.pointer(pointer)?.as_object()?;
    ["maxres", "standard", "high", "medium", "default"]
        .iter()
        .find_map(|quality| {
            thumbnails
                .get(*quality)?
                .get("url")?
                .as_str()
                .map(String::from)
        })
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

fn continuation_token(container: &Value) -> Option<String> {
    container
        .get("continuations")
        .and_then(Value::as_array)
        .and_then(|continuations| {
            continuations.iter().find_map(|continuation| {
                continuation
                    .get("nextContinuationData")
                    .or_else(|| continuation.get("reloadContinuationData"))
                    .and_then(|data| data.get("continuation"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
        })
}

fn runs_text(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|run| run.get("text").and_then(Value::as_str).map(String::from))
        .collect()
}

fn playlist_from_music_item(item: &Value) -> Option<PlaylistSummary> {
    let data = item.get("musicTwoRowItemRenderer")?;
    let browse_id = data
        .pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)
        .or_else(|| {
            data.pointer("/title/runs/0/navigationEndpoint/watchPlaylistEndpoint/playlistId")
                .and_then(Value::as_str)
        })?;
    let id = browse_id
        .strip_prefix("VL")
        .unwrap_or(browse_id)
        .to_string();
    if id.is_empty() {
        return None;
    }
    let subtitle = runs_text(data, "/subtitle/runs");
    let description = subtitle.join("");
    let item_count = subtitle
        .iter()
        .find(|text| text.contains("song") || text.contains("track"))
        .and_then(|text| {
            text.chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0);
    Some(PlaylistSummary {
        id,
        title: data
            .pointer("/title/runs/0/text")
            .and_then(Value::as_str)
            .unwrap_or("Playlist")
            .to_string(),
        description,
        thumbnail: data
            .pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(Value::as_array)
            .and_then(|thumbnails| thumbnails.last())
            .and_then(|thumbnail| thumbnail.get("url"))
            .and_then(Value::as_str)
            .map(String::from),
        item_count,
    })
}

fn track_from_music_item(item: &Value) -> Option<LibraryTrack> {
    let data = item.get("musicResponsiveListItemRenderer")?;
    let video_id = data
        .pointer("/playlistItemData/videoId")
        .or_else(|| {
            data.pointer(
                "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
            )
        })
        .and_then(Value::as_str)?
        .to_string();
    let title = data
        .pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
        .and_then(Value::as_str)?;
    let metadata = data
        .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text/runs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let artist = metadata
        .iter()
        .find(|run| {
            run.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(kind, "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL")
                })
        })
        .and_then(|run| run.get("text"))
        .and_then(Value::as_str)
        .or_else(|| {
            metadata
                .iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .find(|text| *text != "•" && !text.contains(':'))
        })
        .unwrap_or_default()
        .to_string();
    let duration = (1..data
        .pointer("/flexColumns")
        .and_then(Value::as_array)
        .map_or(0, Vec::len))
        .filter_map(|index| {
            data.pointer(&format!(
                "/flexColumns/{index}/musicResponsiveListItemFlexColumnRenderer/text/runs"
            ))
            .and_then(Value::as_array)
        })
        .flatten()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .find(|text| {
            text.split(':').count() > 1
                && text
                    .split(':')
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        })
        .unwrap_or_default()
        .to_string();
    let duration_ms = parse_music_duration(&duration);
    Some(LibraryTrack {
        video_id,
        title: title.to_string(),
        artist,
        duration,
        duration_ms,
        thumbnail: data
            .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
            .and_then(Value::as_array)
            .and_then(|thumbnails| thumbnails.last())
            .and_then(|thumbnail| thumbnail.get("url"))
            .and_then(Value::as_str)
            .map(String::from),
    })
}

fn merge_playlists(
    primary: Vec<PlaylistSummary>,
    fallback: Vec<PlaylistSummary>,
) -> Vec<PlaylistSummary> {
    let mut seen = HashSet::new();
    primary
        .into_iter()
        .chain(fallback)
        .filter(|playlist| seen.insert(playlist.id.clone()))
        .collect()
}

fn parse_music_duration(value: &str) -> u64 {
    let parts: Vec<u64> = value
        .split(':')
        .filter_map(|part| part.parse().ok())
        .collect();
    match parts.as_slice() {
        [minutes, seconds] => (minutes * 60 + seconds) * 1000,
        [hours, minutes, seconds] => (hours * 3600 + minutes * 60 + seconds) * 1000,
        _ => 0,
    }
}

fn track_from_video(item: &Value) -> LibraryTrack {
    let duration = text_at(item, "/contentDetails/duration");
    let duration_ms = parse_iso_duration(&duration);
    LibraryTrack {
        video_id: text_at(item, "/id"),
        title: text_at(item, "/snippet/title"),
        artist: text_at(item, "/snippet/channelTitle"),
        duration: format_duration(duration_ms),
        duration_ms,
        thumbnail: thumbnail_at(item, "/snippet/thumbnails"),
    }
}

fn parse_iso_duration(value: &str) -> u64 {
    let mut total = 0_u64;
    let mut number = String::new();
    for character in value.trim_start_matches("PT").chars() {
        if character.is_ascii_digit() {
            number.push(character);
        } else if !number.is_empty() {
            let amount = number.parse::<u64>().unwrap_or(0);
            total += match character {
                'H' => amount * 3600,
                'M' => amount * 60,
                'S' => amount,
                _ => 0,
            };
            number.clear();
        }
    }
    total * 1000
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::{AuthManager, OAuthFile, URL_SAFE_NO_PAD};
    use base64::Engine;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    #[test]
    fn pkce_s256_matches_rfc7636() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn legacy_oauth_config_remains_compatible() {
        let config: OAuthFile = serde_json::from_value(serde_json::json!({
            "installed": { "client_id": "test", "client_secret": "test", "token_uri": "https://oauth2.googleapis.com/token" }
        })).unwrap();
        assert!(config.installed.redirect_uris.is_empty());
    }

    #[test]
    fn desktop_config_preserves_loopback_redirect() {
        let config: OAuthFile = serde_json::from_value(serde_json::json!({
            "installed": { "client_id": "test", "client_secret": "test", "token_uri": "https://oauth2.googleapis.com/token", "redirect_uris": ["http://localhost"] }
        })).unwrap();
        assert_eq!(config.installed.redirect_uris, vec!["http://localhost"]);
    }

    #[tokio::test]
    #[ignore = "requires a locally signed-in Google account"]
    async fn music_library_probe() {
        let manager = AuthManager::new().expect("OAuth configuration");
        let token = manager.access_token().await.expect("access token");
        let token_info = reqwest::Client::new()
            .get("https://oauth2.googleapis.com/tokeninfo")
            .query(&[("access_token", token)])
            .send()
            .await
            .expect("tokeninfo request")
            .json::<Value>()
            .await
            .expect("tokeninfo json");
        eprintln!("token_scope={:?}", token_info.get("scope"));
        eprintln!(
            "token_audience_matches={:?}",
            token_info
                .get("aud")
                .and_then(Value::as_str)
                .map(|aud| aud == manager.credentials.client_id)
        );

        for playlist_id in ["LM", "LL"] {
            match manager
                .youtube_get(
                    "playlistItems",
                    &[
                        ("part", "contentDetails"),
                        ("playlistId", playlist_id),
                        ("maxResults", "1"),
                    ],
                )
                .await
            {
                Ok(response) => eprintln!(
                    "data_api_{playlist_id}={:?}",
                    response
                        .get("pageInfo")
                        .and_then(|info| info.get("totalResults"))
                ),
                Err(error) => eprintln!("data_api_{playlist_id}=error {error}"),
            }
        }

        for browse_id in ["VLLM", "FEmusic_liked_videos", "FEmusic_liked_playlists"] {
            match manager.youtube_music_browse(browse_id, None).await {
                Ok(response) => eprintln!(
                    "{browse_id}=ok keys={:?}",
                    response
                        .as_object()
                        .map(|value| value.keys().collect::<Vec<_>>())
                ),
                Err(error) => eprintln!("{browse_id}=error {error}"),
            }
        }

        match manager.liked_songs().await {
            Ok(tracks) => eprintln!("resolved_liked_count={}", tracks.len()),
            Err(error) => eprintln!("resolved_liked_error={error}"),
        }
        match manager.playlists_data_api().await {
            Ok(playlists) => eprintln!("resolved_playlist_count={}", playlists.len()),
            Err(error) => eprintln!("resolved_playlist_error={error}"),
        }
    }
}
