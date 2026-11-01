<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { openUrl } from "@tauri-apps/plugin-opener";

  type TrackResult = {
    video_id: string;
    title: string;
    artist: string;
    duration: string;
    duration_ms: number;
    thumbnail?: string;
    artist_id?: string;
    album_id?: string;
    album?: string;
  };

  type ShelfItem = { id: string; title: string; subtitle: string; thumbnail?: string };
  type ArtistPage = { name: string; thumbnail?: string; top_songs: TrackResult[]; albums: ShelfItem[] };
  type AlbumPage = { title: string; artist: string; thumbnail?: string; tracks: TrackResult[] };

  type PlaybackState = {
    status: "playing" | "paused" | "stopped";
    position_ms: number;
    duration_ms: number;
    path: string;
  };

  type LyricsResult = {
    syncedLyrics?: string;
    plainLyrics?: string;
  };

  type LyricLine = {
    time_ms: number | null;
    text: string;
  };

  type AccountProfile = {
    name: string;
    avatar?: string;
    channel_id: string;
  };

  type AuthStatus = {
    signed_in: boolean;
    profile?: AccountProfile;
  };

  type DeviceAuthPrompt = {
    verification_url: string;
    user_code: string;
  };

  type PlaylistSummary = {
    id: string;
    title: string;
    description: string;
    thumbnail?: string;
    item_count: number;
  };

  type View = "home" | "search" | "liked" | "playlists" | "playlist" | "recent" | "artist" | "album" | "explore";

  let query = $state("");
  let suggestions = $state<TrackResult[]>([]);
  let searchFocused = $state(false);
  let suggestionIndex = $state(-1);
  let suggestionRequest = 0;
  let activeView = $state<View>("home");
  let auth = $state<AuthStatus>({ signed_in: false });
  let authPrompt = $state<DeviceAuthPrompt | null>(null);
  let authCodeCopied = $state(false);
  let accountLoading = $state(false);
  let results = $state<TrackResult[]>([]);
  let homeTracks = $state<TrackResult[]>([]);
  let homeLoaded = $state(false);
  let libraryTracks = $state<TrackResult[]>([]);
  const RECENT_KEY = "desktube-recent-tracks";
  let recentTracks = $state<TrackResult[]>(loadRecent());
  let likedIds = $state<Set<string>>(new Set());
  let playlists = $state<PlaylistSummary[]>([]);
  let selectedPlaylist = $state<PlaylistSummary | null>(null);
  let queue = $state<TrackResult[]>([]);
  let currentIndex = $state(-1);
  let searching = $state(false);
  let loadingTrack = $state(false);
  let playRequest = 0;
  let error = $state("");
  let volume = $state(0.8);
  let muted = $state(false);
  let sidebarCollapsed = $state(false);
  let nowPlayingExpanded = $state(false);
  let focusMode = $state(false);
  let lyricsLoading = $state(false);
  let lyricsMessage = $state("");
  let lyricLines = $state<LyricLine[]>([]);
  let lyricsTrackId = $state("");
  let scrubbing = $state(false);
  let scrubPosition = $state(0);
  let artistPage = $state<ArtistPage | null>(null);
  let albumPage = $state<AlbumPage | null>(null);
  let viewHistory = $state<View[]>([]);
  let radioMode = $state(localStorage.getItem("desktube-radio") === "1");
  let radioLoading = $state(false);
  let crossfade = $state(localStorage.getItem("desktube-crossfade") !== "0");
  let nextQueuedFor = $state("");
  let expectRollover = false;
  let settingsOpen = $state(false);
  let remoteOpen = $state(false);
  let remoteInfo = $state<{ url: string; qr_svg?: string } | null>(null);
  let audioDevices = $state<string[]>([]);
  let audioDevice = $state(localStorage.getItem("desktube-audio-device") ?? "");
  let sleepEndAt = $state<number | null>(null);
  let sleepFading = $state(false);
  let sleepRemainingMs = $state(0);
  let npAccent = $state<string | null>(null);
  let shuffleMode = $state(localStorage.getItem("desktube-shuffle") === "1");
  let originalQueue: TrackResult[] = [];
  const SESSION_KEY = "desktube-session";
  const MOODS = [
    { label: "Relaxing", query: "relaxing music playlist", color: "#8ab4f8", keywords: ["relax", "chill", "calm", "acoustic", "ambient", "mellow", "soft", "sleep", "lullab"] },
    { label: "Pop", query: "pop hits", color: "#ff7eb6", keywords: ["pop"] },
    { label: "Rap", query: "rap hits", color: "#b087ff", keywords: ["rap", "hip hop", "hip-hop", "drill", "trap"] },
    { label: "Rock", query: "rock classics", color: "#ff6b57", keywords: ["rock", "metal", "punk", "grunge", "nirvana", "metallica", "queen", "zeppelin", "foo fighters", "linkin park"] },
    { label: "Lo-fi", query: "lofi beats", color: "#e0b36a", keywords: ["lofi", "lo-fi", "lo fi", "chillhop", "chill hop"] },
    { label: "Workout", query: "workout music", color: "#ff9d5c", keywords: ["workout", "gym", "pump", "beast mode", "power"] },
    { label: "Focus", query: "deep focus music", color: "#5ad8c2", keywords: ["focus", "study", "concentration", "instrumental", "piano", "classical", "ambient"] },
    { label: "Throwback", query: "2000s hits", color: "#f8cf5d", keywords: ["throwback", "2000", "90s", "80s", "70s", "oldie", "classic"] },
  ];
  let activeMood = $state<string | null>(null);
  let moodTracks = $state<TrackResult[]>([]);
  let moodLoading = $state(false);
  let moodRequest = 0;
  let gridTracks = $derived(activeMood ? moodTracks : homeTracks.slice(0, 6));
  let exploreTracks = $state<TrackResult[]>([]);
  let exploreLoading = $state(false);
  let exploreSeeds = $state<string[]>([]);
  let playback = $state<PlaybackState>({ status: "stopped", position_ms: 0, duration_ms: 0, path: "" });
  let currentTrack = $derived(currentIndex >= 0 ? queue[currentIndex] : null);
  let heroTrack = $derived(currentTrack ?? homeTracks[0] ?? null);
  let visibleTracks = $derived(
    activeView === "search"
      ? results
      : activeView === "liked" || activeView === "playlist"
        ? libraryTracks
        : activeView === "recent"
          ? recentTracks
          : activeView === "explore"
            ? exploreTracks
            : activeView === "artist"
            ? (artistPage?.top_songs ?? [])
            : activeView === "album"
              ? (albumPage?.tracks ?? [])
              : [],
  );
  let pageTitle = $derived(
    activeView === "liked"
      ? "Liked songs"
      : activeView === "playlist"
        ? selectedPlaylist?.title ?? "Playlist"
        : activeView === "recent"
          ? "Recently played"
          : activeView === "explore"
            ? "Made for you"
            : activeView === "artist"
            ? (artistPage?.name ?? "Artist")
            : activeView === "album"
              ? (albumPage?.title ?? "Album")
              : activeView === "playlists"
            ? "Your playlists"
          : activeView === "search" && results.length
            ? `Results for “${query}”`
            : "Welcome to DeskTube",
  );
  let pageSubtitle = $derived(
    activeView === "liked"
      ? `${libraryTracks.length} liked music videos`
      : activeView === "playlist"
        ? `${libraryTracks.length} tracks`
        : activeView === "recent"
          ? `${recentTracks.length} tracks played on DeskTube`
          : activeView === "explore"
            ? `${exploreTracks.length} recommendations from your taste`
            : activeView === "artist"
            ? `${artistPage?.top_songs.length ?? 0} top songs`
            : activeView === "album"
              ? `${albumPage?.artist ?? ""} · ${albumPage?.tracks.length ?? 0} tracks`
              : activeView === "playlists"
            ? `${playlists.length} playlists from YouTube Music`
          : activeView === "search" && results.length
            ? `${results.length} songs from YouTube Music`
            : auth.signed_in
              ? `Signed in as ${auth.profile?.name ?? "Google user"}`
              : "Search freely or connect Google to access your library.",
  );
  let timelineDuration = $derived(playback.duration_ms || currentTrack?.duration_ms || 0);
  let activeLyricIndex = $derived.by(() => {
    let active = -1;
    for (let index = 0; index < lyricLines.length; index += 1) {
      const time = lyricLines[index].time_ms;
      if (time !== null && time <= playback.position_ms + 250) active = index;
    }
    return active;
  });

  $effect(() => {
    let previousStatus = playback.status;
    let previousPath = playback.path;
    const unlisten = listen<PlaybackState>("playback-state", (event) => {
      const nextState = event.payload;
      const duration = nextState.duration_ms || currentTrack?.duration_ms || 0;

      // Gapless/crossfade rollover: the engine reports which file is playing;
      // a path change means the queued next track took over.
      if (
        previousStatus === "playing" &&
        nextState.status === "playing" &&
        nextState.path !== "" &&
        nextState.path !== previousPath &&
        expectRollover &&
        currentIndex < queue.length - 1
      ) {
        expectRollover = false;
        void advanceToQueuedTrack();
      }

      // Track ended with nothing queued — fall back to manual advance.
      if (previousStatus === "playing" && nextState.status === "stopped" && !loadingTrack) {
        void nextTrack();
      }

      // Near the end? Pre-load the next track into the audio engine so the
      // transition is gapless (or a crossfade when enabled).
      if (
        nextState.status === "playing" &&
        duration > 0 &&
        !loadingTrack &&
        currentIndex >= 0
      ) {
        const next = queue[currentIndex + 1];
        const threshold = crossfade ? 6500 : 9000;
        if (next && duration - nextState.position_ms <= threshold && nextQueuedFor !== next.video_id) {
          nextQueuedFor = next.video_id;
          void invoke("queue_next_track", {
            videoId: next.video_id,
            durationMs: next.duration_ms,
            crossfadeMs: crossfade ? 4000 : 0,
          })
            .then(() => {
              if (nextQueuedFor === next.video_id) expectRollover = true;
            })
            .catch(() => {
              if (nextQueuedFor === next.video_id) nextQueuedFor = "";
            });
        } else if (!next && radioMode && !radioLoading) {
          void extendRadio();
        }
      }

      // Sleep timer: fade the last 15 s, then pause and restore volume.
      if (sleepEndAt !== null) {
        const remaining = sleepEndAt - Date.now();
        sleepRemainingMs = Math.max(0, remaining);
        if (remaining <= 15000 && !sleepFading) sleepFading = true;
        if (sleepFading && remaining > 0) {
          void invoke("set_volume", { volume: Math.max(0.001, (remaining / 15000) * volume) });
        } else if (remaining <= 0) {
          sleepEndAt = null;
          sleepFading = false;
          void invoke("pause").then(() => invoke("set_volume", { volume }));
        }
      }

      // Persist session so the app can resume where it left off.
      try {
        localStorage.setItem(
          SESSION_KEY,
          JSON.stringify({ queue, index: currentIndex, position_ms: nextState.position_ms }),
        );
      } catch { /* storage full — skip */ }

      previousStatus = nextState.status;
      previousPath = nextState.path;
      playback = nextState;
      if (!scrubbing) scrubPosition = nextState.position_ms;
    });
    return () => void unlisten.then((stop) => stop());
  });

  $effect(() => {
    const id = currentTrack?.video_id;
    if (id && currentTrack) extractAccent(currentTrack);
  });

  $effect(() => {
    void refreshAuth();
    void restoreSession();
    void loadAudioDevices().then(() => {
      if (audioDevice) void invoke("set_audio_device", { name: audioDevice }).catch(() => undefined);
    });
    const authListener = listen<boolean>("auth-updated", (event) => {
      if (event.payload === true) authPrompt = null;
      void refreshAuth();
    });
    const authErrorListener = listen<string>("auth-error", (event) => {
      authPrompt = null;
      error = event.payload;
    });
    let lastMediaCommand = "";
    let lastMediaTime = 0;
    const mediaListener = listen<"toggle" | "next" | "previous">("media-command", (event) => {
      const now = Date.now();
      // Hardware keys fire via both the global-shortcut plugin and SMTC.
      if (event.payload === lastMediaCommand && now - lastMediaTime < 600) return;
      lastMediaCommand = event.payload;
      lastMediaTime = now;
      if (event.payload === "toggle") void togglePause();
      if (event.payload === "next") void nextTrack();
      if (event.payload === "previous") void previousTrack();
    });
    const libraryListener = listen<TrackResult[]>("library-updated", (event) => {
      likedIds = new Set(event.payload.map((track) => track.video_id));
      libraryTracks = event.payload;
      homeTracks = event.payload.slice(0, 12);
    });
    // Phone remote commands forwarded by the LAN server.
    const remoteListener = listen<{ action: string; position_ms?: number; volume?: number }>(
      "remote-command",
      (event) => {
        const { action, position_ms, volume: v } = event.payload;
        if (action === "toggle") void togglePause();
        else if (action === "next") void nextTrack();
        else if (action === "previous") void previousTrack();
        else if (action === "seek" && position_ms !== undefined) void seekTo(position_ms);
        else if (action === "volume" && v !== undefined) void applyVolume(v);
      },
    );
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (settingsOpen) settingsOpen = false;
        else if (remoteOpen) remoteOpen = false;
        else if (focusMode) void setFocusMode(false);
        return;
      }
      // Don't hijack typing in inputs/selects.
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, select, [contenteditable='true']")) return;
      const ctrl = event.ctrlKey || event.metaKey;
      if (event.key === " ") {
        event.preventDefault();
        void togglePause();
      } else if (ctrl && event.key === "ArrowRight") {
        void nextTrack();
      } else if (ctrl && event.key === "ArrowLeft") {
        void previousTrack();
      } else if (event.key === "ArrowRight") {
        void seekBy(10000);
      } else if (event.key === "ArrowLeft") {
        void seekBy(-10000);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        void applyVolume(volume + 0.05);
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        void applyVolume(volume - 0.05);
      } else if (event.key === "/") {
        event.preventDefault();
        void navigate("search");
      } else if (event.key.toLowerCase() === "m") {
        void toggleMute();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      void authListener.then((stop) => stop());
      void authErrorListener.then((stop) => stop());
      void mediaListener.then((stop) => stop());
      void libraryListener.then((stop) => stop());
      void remoteListener.then((stop) => stop());
    };
  });

  $effect(() => {
    if (!nowPlayingExpanded && focusMode) void setFocusMode(false);
  });

  async function setFocusMode(active: boolean) {
    focusMode = active;
    try {
      await getCurrentWindow().setFullscreen(active);
    } catch {
      /* fullscreen unsupported — keep CSS-only focus mode */
    }
  }

  $effect(() => {
    const term = query.trim();
    suggestionIndex = -1;
    if (term.length < 2) {
      suggestions = [];
      return;
    }
    const request = ++suggestionRequest;
    const timer = window.setTimeout(async () => {
      try {
        const matches = await invoke<TrackResult[]>("search_tracks", { query: term });
        if (request === suggestionRequest && query.trim() === term) suggestions = matches.slice(0, 10);
      } catch {
        if (request === suggestionRequest) suggestions = [];
      }
    }, 280);
    return () => window.clearTimeout(timer);
  });

  $effect(() => {
    if (nowPlayingExpanded && activeLyricIndex >= 0) {
      document.getElementById(`lyric-${activeLyricIndex}`)?.scrollIntoView({
        behavior: "smooth",
        block: "center",
      });
    }
  });

  async function refreshAuth() {
    try {
      auth = await invoke<AuthStatus>("auth_status");
      if (auth.signed_in && !homeLoaded) await loadHome();
    } catch {
      auth = { signed_in: false };
    }
  }

  async function loadHome() {
    accountLoading = true;
    try {
      const [tracks, accountPlaylists] = await Promise.all([
        invoke<TrackResult[]>("liked_songs"),
        invoke<PlaylistSummary[]>("account_playlists"),
      ]);
      homeTracks = tracks.slice(0, 12);
      libraryTracks = tracks;
      likedIds = new Set(tracks.map((item) => item.video_id));
      playlists = accountPlaylists;
      homeLoaded = true;
    } catch (e) {
      error = String(e);
    } finally {
      accountLoading = false;
    }
  }

  async function signIn() {
    accountLoading = true;
    error = "";
    try {
      authPrompt = await invoke<DeviceAuthPrompt>("begin_google_auth");
      authCodeCopied = false;
      await openUrl(authPrompt.verification_url);
    } catch (e) {
      authPrompt = null;
      authCodeCopied = false;
      error = String(e);
    } finally {
      accountLoading = false;
    }
  }

  async function copyAuthCode() {
    if (!authPrompt) return;
    try {
      await navigator.clipboard.writeText(authPrompt.user_code);
      authCodeCopied = true;
    } catch {
      error = "Could not copy the Google sign-in code.";
    }
  }

  async function signOut() {
    await invoke("sign_out");
    auth = { signed_in: false };
    authPrompt = null;
    authCodeCopied = false;
    activeView = "home";
    homeTracks = [];
    homeLoaded = false;
    libraryTracks = [];
    playlists = [];
  }

  async function navigate(view: View) {
    activeView = view;
    error = "";
    if (view === "search") {
      setTimeout(() => document.querySelector<HTMLInputElement>(".search-input")?.focus());
      return;
    }
    if (view === "home" && auth.signed_in && !homeLoaded) {
      await loadHome();
      return;
    }
    if (!auth.signed_in && (view === "liked" || view === "playlists")) return;
    if (view === "explore") {
      void buildExplore();
      return;
    }
    if (view === "recent" || view === "home") return;
    accountLoading = true;
    try {
      if (view === "liked") libraryTracks = await invoke<TrackResult[]>("liked_songs");
      if (view === "playlists") playlists = await invoke<PlaylistSummary[]>("account_playlists");
    } catch (e) {
      error = String(e);
    } finally {
      accountLoading = false;
    }
  }

  async function openPlaylist(playlist: PlaylistSummary) {
    selectedPlaylist = playlist;
    activeView = "playlist";
    accountLoading = true;
    try {
      libraryTracks = await invoke<TrackResult[]>("playlist_items", { playlistId: playlist.id });
    } catch (e) {
      error = String(e);
    } finally {
      accountLoading = false;
    }
  }

  async function createNewPlaylist() {
    if (!auth.signed_in) return;
    const title = window.prompt("Create a private playlist in your YouTube account. Playlist name:");
    if (!title?.trim()) return;
    try {
      const playlist = await invoke<PlaylistSummary>("create_playlist", { title: title.trim() });
      playlists = [playlist, ...playlists];
      activeView = "playlists";
    } catch (e) {
      error = String(e);
    }
  }

  async function search() {
    if (!query.trim()) return;
    activeView = "search";
    searchFocused = false;
    searching = true;
    error = "";
    try {
      results = await invoke<TrackResult[]>("search_tracks", { query: query.trim() });
    } catch (e) {
      error = String(e);
    } finally {
      searching = false;
    }
  }

  async function chooseSuggestion(track: TrackResult) {
    query = track.title;
    results = [...suggestions];
    activeView = "search";
    searchFocused = false;
    queue = [...suggestions];
    await playAt(queue.findIndex((item) => item.video_id === track.video_id));
  }

  function handleSearchKey(event: KeyboardEvent) {
    if (!searchFocused || !suggestions.length) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      suggestionIndex = Math.min(suggestions.length - 1, suggestionIndex + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      suggestionIndex = Math.max(0, suggestionIndex - 1);
    } else if (event.key === "Enter" && suggestionIndex >= 0) {
      event.preventDefault();
      void chooseSuggestion(suggestions[suggestionIndex]);
    } else if (event.key === "Escape") {
      searchFocused = false;
    }
  }

  /// Insert a track immediately after the current one in the queue;
  /// moves it if it's already queued further ahead.
  function playNext(track: TrackResult) {
    if (currentIndex < 0 || !queue.length) {
      queue = [track];
      void playAt(0);
      return;
    }
    if (queue[currentIndex]?.video_id === track.video_id) return;
    const q = [...queue];
    const existing = q.findIndex((item, i) => i > currentIndex && item.video_id === track.video_id);
    if (existing >= 0) q.splice(existing, 1);
    q.splice(currentIndex + 1, 0, track);
    queue = q;
  }

  async function playResult(track: TrackResult) {
    queue = [...visibleTracks];
    await playAt(queue.findIndex((item) => item.video_id === track.video_id));
  }

  async function playHomeTrack(track: TrackResult) {
    queue = [...homeTracks];
    await playAt(queue.findIndex((item) => item.video_id === track.video_id));
  }

  function shuffle<T>(items: T[]): T[] {
    const copy = [...items];
    for (let index = copy.length - 1; index > 0; index -= 1) {
      const swap = Math.floor(Math.random() * (index + 1));
      [copy[index], copy[swap]] = [copy[swap], copy[index]];
    }
    return copy;
  }

  /// Recommendations: seed YouTube's "radio" similarity graph from your
  /// most-played recent tracks plus a few random liked songs, then rank
  /// candidates by how many seed mixes they appear in (with a bonus for
  /// artists already in your library).
  async function buildExplore() {
    if (exploreLoading) return;
    exploreLoading = true;
    error = "";
    try {
      if (auth.signed_in && libraryTracks.length === 0) {
        try {
          libraryTracks = await invoke<TrackResult[]>("liked_songs");
        } catch {
          /* proceed with recents only */
        }
      }
      // Seeds: top-3 most repeated recent tracks + 3 random liked tracks.
      const freq = new Map<string, number>();
      for (const t of recentTracks) freq.set(t.video_id, (freq.get(t.video_id) ?? 0) + 1);
      const topRecent = [...freq.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 3)
        .map(([id]) => id);
      const seedIds = new Set<string>(topRecent);
      for (const t of shuffle(libraryTracks).slice(0, 3)) seedIds.add(t.video_id);
      for (const t of recentTracks) if (seedIds.size < 4) seedIds.add(t.video_id);
      const seeds = [...seedIds].slice(0, 6);
      exploreSeeds = seeds;
      if (seeds.length === 0) {
        exploreTracks = [];
        return;
      }

      // Artists you already like (normalize "X - Topic" → "x").
      const likedArtists = new Set(
        libraryTracks.map((t) => t.artist.replace(/\s*-\s*topic$/i, "").toLowerCase()),
      );
      const seen = new Set([...likedIds, ...recentTracks.map((t) => t.video_id)]);
      const scored = new Map<string, { track: TrackResult; score: number }>();
      await Promise.all(
        seeds.map(async (videoId) => {
          try {
            const mix = await invoke<TrackResult[]>("radio_tracks", { videoId });
            for (const track of mix) {
              if (seen.has(track.video_id)) continue;
              const artistBoost = likedArtists.has(
                track.artist.replace(/\s*-\s*topic$/i, "").toLowerCase(),
              )
                ? 2
                : 0;
              const entry = scored.get(track.video_id) ?? { track, score: 0 };
              entry.score += 10 + artistBoost;
              scored.set(track.video_id, entry);
            }
          } catch {
            /* a failed seed just contributes nothing */
          }
        }),
      );
      exploreTracks = [...scored.values()]
        .sort((a, b) => b.score - a.score)
        .slice(0, 30)
        .map((entry) => entry.track);
    } catch (e) {
      error = String(e);
    } finally {
      exploreLoading = false;
    }
  }

  async function selectMood(mood: { label: string; query: string; keywords: string[] }) {
    if (activeMood === mood.label) {
      activeMood = null;
      moodTracks = [];
      return;
    }
    activeMood = mood.label;
    const request = ++moodRequest;
    moodLoading = true;
    try {
      let library: TrackResult[] = [];
      if (auth.signed_in) {
        try {
          library = libraryTracks.length ? libraryTracks : await invoke<TrackResult[]>("liked_songs");
        } catch {
          library = [];
        }
      }
      const matches = shuffle(
        library.filter((track) => {
          const text = `${track.title} ${track.artist}`.toLowerCase();
          return mood.keywords.some((keyword) => text.includes(keyword));
        }),
      );
      const seen = new Set(matches.map((track) => track.video_id));
      const extras =
        matches.length >= 12
          ? []
          : (await invoke<TrackResult[]>("search_tracks", { query: mood.query })).filter(
              (track) => !seen.has(track.video_id),
            );
      if (request === moodRequest) moodTracks = [...matches, ...extras].slice(0, 12);
    } catch (e) {
      if (request === moodRequest) error = String(e);
    } finally {
      if (request === moodRequest) moodLoading = false;
    }
  }

  function openMoodSearch() {
    const mood = MOODS.find((item) => item.label === activeMood);
    if (!mood) return;
    query = mood.query;
    void search();
  }

  async function playGridTrack(track: TrackResult) {
    queue = [...gridTracks];
    await playAt(queue.findIndex((item) => item.video_id === track.video_id));
  }

  function prefetchAround(index: number) {
    for (const target of [index - 1, index + 1, index + 2]) {
      const track = queue[target];
      if (track) void invoke("prepare_track", { videoId: track.video_id }).catch(() => undefined);
    }
  }

  async function playAt(index: number) {
    if (index < 0 || index >= queue.length) return;
    const request = ++playRequest;
    currentIndex = index;
    nextQueuedFor = "";
    expectRollover = false;
    loadingTrack = true;
    error = "";
    prefetchAround(index);
    try {
      const track = queue[index];
      pushRecent(track);
      await invoke("play_track", {
        videoId: track.video_id,
        durationMs: track.duration_ms,
        title: track.title,
        artist: track.artist,
        thumbnail: track.thumbnail ?? null,
      });
      if (request === playRequest && nowPlayingExpanded) await loadLyrics(track);
    } catch (e) {
      if (request === playRequest) error = String(e);
    } finally {
      if (request === playRequest) loadingTrack = false;
    }
  }

  async function togglePause() {
    if (!currentTrack || loadingTrack) return;
    await invoke(playback.status === "playing" ? "pause" : "resume");
  }

  async function previousTrack() {
    if (!queue.length || loadingTrack) return;
    const target = playback.position_ms > 3000 ? currentIndex : Math.max(0, currentIndex - 1);
    await playAt(target);
  }

  async function nextTrack() {
    if (!queue.length || loadingTrack) return;
    if (currentIndex >= queue.length - 1) {
      if (radioMode && currentTrack) await extendRadio();
      if (currentIndex >= queue.length - 1) return;
    }
    await playAt(currentIndex + 1);
  }

  /// The queued gapless/crossfade source just became current — update
  /// metadata, lyrics, recents, and prefetch without re-invoking play_track.
  async function advanceToQueuedTrack() {
    if (currentIndex >= queue.length - 1) return;
    currentIndex += 1;
    const track = queue[currentIndex];
    pushRecent(track);
    prefetchAround(currentIndex);
    void invoke("announce_track", {
      title: track.title,
      artist: track.artist,
      thumbnail: track.thumbnail ?? null,
    }).catch(() => undefined);
    if (nowPlayingExpanded && lyricsTrackId !== track.video_id) await loadLyrics(track);
  }

  async function extendRadio() {
    if (!currentTrack || radioLoading) return;
    radioLoading = true;
    try {
      const tracks = await invoke<TrackResult[]>("radio_tracks", { videoId: currentTrack.video_id });
      const seen = new Set(queue.map((item) => item.video_id));
      const fresh = tracks.filter((track) => !seen.has(track.video_id));
      if (fresh.length) queue = [...queue, ...fresh];
    } catch {
      /* radio unavailable — playback just ends at queue end */
    } finally {
      radioLoading = false;
    }
  }

  async function toggleRadio() {
    radioMode = !radioMode;
    localStorage.setItem("desktube-radio", radioMode ? "1" : "0");
    if (radioMode) await extendRadio();
  }

  /// Restore the previous session's queue and resume playback at the
  /// saved position. Runs once at launch.
  let sessionRestored = false;
  async function restoreSession() {
    if (sessionRestored) return;
    sessionRestored = true;
    try {
      const saved = JSON.parse(localStorage.getItem(SESSION_KEY) ?? "null") as
        | { queue: TrackResult[]; index: number; position_ms: number }
        | null;
      if (!saved?.queue?.length || saved.index < 0 || saved.index >= saved.queue.length) return;
      queue = saved.queue;
      await playAt(saved.index);
      // Restore paused at the saved position rather than auto-playing.
      await invoke("pause");
      if (saved.position_ms > 2000) {
        // Give the decoder a moment before seeking.
        setTimeout(() => void seekTo(saved.position_ms), 400);
      }
    } catch {
      /* corrupted session — start fresh */
    }
  }

  /// Spotify-style shuffle: keeps the current track first and shuffles
  /// the rest; toggling off restores the original order.
  function toggleShuffle() {
    if (queue.length < 2) return;
    shuffleMode = !shuffleMode;
    localStorage.setItem("desktube-shuffle", shuffleMode ? "1" : "0");
    const current = queue[currentIndex];
    if (shuffleMode) {
      originalQueue = [...queue];
      queue = [current, ...shuffle(queue.filter((_, i) => i !== currentIndex))];
      currentIndex = 0;
    } else {
      queue = originalQueue.length ? originalQueue : [...queue];
      currentIndex = Math.max(0, queue.findIndex((t) => t.video_id === current.video_id));
      originalQueue = [];
    }
  }

  async function setCrossfade(enabled: boolean) {
    crossfade = enabled;
    localStorage.setItem("desktube-crossfade", enabled ? "1" : "0");
  }

  async function loadAudioDevices() {
    try {
      audioDevices = await invoke<string[]>("audio_devices");
    } catch {
      audioDevices = [];
    }
  }

  async function pickAudioDevice(name: string) {
    audioDevice = name;
    localStorage.setItem("desktube-audio-device", name);
    try {
      await invoke("set_audio_device", { name: name || null });
    } catch (e) {
      error = String(e);
    }
  }

  function setSleepTimer(minutes: number) {
    sleepEndAt = minutes > 0 ? Date.now() + minutes * 60_000 : null;
    sleepFading = false;
  }

  async function openArtist(track: TrackResult) {
    if (!track.artist_id) {
      query = track.artist;
      await search();
      return;
    }
    viewHistory = [...viewHistory, activeView];
    activeView = "artist";
    artistPage = null;
    accountLoading = true;
    try {
      artistPage = await invoke<ArtistPage>("artist_page", { browseId: track.artist_id });
    } catch (e) {
      error = String(e);
      activeView = viewHistory[viewHistory.length - 1] ?? "home";
      viewHistory = viewHistory.slice(0, -1);
    } finally {
      accountLoading = false;
    }
  }

  async function openAlbum(id: string) {
    viewHistory = [...viewHistory, activeView];
    activeView = "album";
    albumPage = null;
    accountLoading = true;
    try {
      albumPage = await invoke<AlbumPage>("album_page", { browseId: id });
    } catch (e) {
      error = String(e);
      activeView = viewHistory[viewHistory.length - 1] ?? "home";
      viewHistory = viewHistory.slice(0, -1);
    } finally {
      accountLoading = false;
    }
  }

  function goBack() {
    const previous = viewHistory[viewHistory.length - 1];
    if (!previous) return;
    viewHistory = viewHistory.slice(0, -1);
    activeView = previous;
  }

  /// Dominant-color theming for the expanded Now Playing view.
  function extractAccent(track: TrackResult) {
    npAccent = null;
    const url = highQualityCover(track);
    const image = new Image();
    image.crossOrigin = "anonymous";
    image.onload = () => {
      try {
        const canvas = document.createElement("canvas");
        canvas.width = 8;
        canvas.height = 8;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        ctx.drawImage(image, 0, 0, 8, 8);
        const data = ctx.getImageData(0, 0, 8, 8).data;
        let r = 0, g = 0, b = 0, weight = 0;
        for (let i = 0; i < data.length; i += 4) {
          const lum = 0.299 * data[i] + 0.587 * data[i + 1] + 0.114 * data[i + 2];
          // Skip near-black/near-white pixels so the tint stays colorful.
          const w = lum > 25 && lum < 230 ? 1 : 0.15;
          r += data[i] * w; g += data[i + 1] * w; b += data[i + 2] * w;
          weight += w;
        }
        if (weight > 0) {
          npAccent = `rgb(${Math.round(r / weight)} ${Math.round(g / weight)} ${Math.round(b / weight)})`;
        }
      } catch {
        npAccent = null;
      }
    };
    image.onerror = () => (npAccent = null);
    image.src = url;
  }

  async function seekTo(positionMs: number) {
    if (!currentTrack || loadingTrack) return;
    const target = Math.max(0, Math.min(positionMs, timelineDuration));
    scrubPosition = target;
    try {
      await invoke("seek", { positionMs: Math.round(target) });
      playback = { ...playback, position_ms: target };
    } catch (e) {
      error = String(e);
    } finally {
      scrubbing = false;
    }
  }

  async function seekBy(deltaMs: number) {
    await seekTo(playback.position_ms + deltaMs);
  }

  async function toggleNowPlaying() {
    if (!currentTrack) return;
    nowPlayingExpanded = !nowPlayingExpanded;
    if (nowPlayingExpanded && lyricsTrackId !== currentTrack.video_id) {
      await loadLyrics(currentTrack);
    }
  }

  async function loadLyrics(track: TrackResult) {
    lyricsLoading = true;
    lyricsMessage = "";
    lyricLines = [];
    lyricsTrackId = track.video_id;
    try {
      const lyrics = await invoke<LyricsResult | null>("fetch_lyrics", {
        videoId: track.video_id,
        title: track.title,
        artist: track.artist,
        durationMs: track.duration_ms,
      });
      if (!lyrics) {
        lyricsMessage = "Lyrics aren't available for this track.";
        return;
      }
      lyricLines = parseLyrics(lyrics);
      if (!lyricLines.length) lyricsMessage = "Lyrics aren't available for this track.";
    } catch {
      lyricsTrackId = "";
      lyricsMessage = "Lyrics are temporarily unavailable.";
    } finally {
      lyricsLoading = false;
    }
  }

  function parseLyrics(lyrics: LyricsResult): LyricLine[] {
    if (lyrics.syncedLyrics) {
      return lyrics.syncedLyrics.split("\n").flatMap((line) => {
        const match = line.match(/^\[(\d+):(\d+(?:\.\d+)?)\]\s*(.*)$/);
        if (!match) return [];
        return [{ time_ms: (Number(match[1]) * 60 + Number(match[2])) * 1000, text: match[3] || "·" }];
      });
    }
    return (lyrics.plainLyrics ?? "")
      .split("\n")
      .filter((line) => line.trim())
      .map((text) => ({ time_ms: null, text }));
  }

  function highQualityCover(track: TrackResult) {
    if (!track.thumbnail) return `https://i.ytimg.com/vi/${track.video_id}/hqdefault.jpg`;
    return track.thumbnail.replace(/=w\d+-h\d+.*$/, "=w1200-h1200-l90-rj");
  }

  function expandedCoverFallback(event: Event, track: TrackResult) {
    const image = event.currentTarget as HTMLImageElement;
    if (!image.dataset.originalTried && track.thumbnail && image.src !== track.thumbnail) {
      image.dataset.originalTried = "true";
      image.src = track.thumbnail;
      return;
    }
    coverFallback(event);
  }

  async function applyVolume(v: number) {
    volume = Math.min(2, Math.max(0, v));
    if (volume > 0 && muted) muted = false;
    await invoke("set_volume", { volume: muted ? 0 : volume });
  }

  async function setVolume(event: Event) {
    await applyVolume(Number((event.target as HTMLInputElement).value));
  }

  async function toggleRemote() {
    remoteOpen = !remoteOpen;
    if (remoteOpen) settingsOpen = false;
    if (remoteOpen && !remoteInfo) {
      try {
        remoteInfo = await invoke<{ url: string; qr_svg?: string } | null>("remote_pairing");
      } catch {
        remoteInfo = null;
      }
    }
  }

  async function toggleMute() {
    muted = !muted;
    if (!muted && volume === 0) volume = 0.8;
    await invoke("set_volume", { volume: muted ? 0 : volume });
  }

  function coverFallback(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    image.onerror = null;
    image.src = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 200 200'%3E%3Ccircle cx='100' cy='100' r='100' fill='%231ed760'/%3E%3Cpath d='M78 52v96l82-48z' fill='white'/%3E%3C/svg%3E";
  }

  function loadRecent(): TrackResult[] {
    try {
      const parsed = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]") as TrackResult[];
      return Array.isArray(parsed) ? parsed.slice(0, 40) : [];
    } catch {
      return [];
    }
  }

  function pushRecent(track: TrackResult) {
    recentTracks = [
      track,
      ...recentTracks.filter((item) => item.video_id !== track.video_id),
    ].slice(0, 40);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(recentTracks));
    } catch {
      /* storage full — keep in-memory only */
    }
  }

  async function toggleLike(track: TrackResult | null) {
    if (!track) return;
    if (!auth.signed_in) {
      error = "Connect Google to like songs.";
      return;
    }
    const liked = likedIds.has(track.video_id);
    const next = new Set(likedIds);
    if (liked) next.delete(track.video_id);
    else next.add(track.video_id);
    likedIds = next;
    try {
      await invoke(liked ? "unlike_track" : "like_track", { videoId: track.video_id });
      if (!liked) libraryTracks = [track, ...libraryTracks];
      else libraryTracks = libraryTracks.filter((item) => item.video_id !== track.video_id);
    } catch (e) {
      likedIds = new Set(libraryTracks.map((item) => item.video_id));
      error = String(e);
    }
  }

  function formatTime(ms: number) {
    const seconds = Math.floor(ms / 1000);
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  }
</script>

<svelte:head><title>DeskTube</title></svelte:head>

<div class:sidebar-collapsed={sidebarCollapsed} class:focus-mode={focusMode} class="app-shell">
  <aside class="sidebar">
    <div class="brand"><span class="brand-mark"><svg viewBox="0 0 32 32" aria-hidden="true"><path d="M12 8.5v15l11-7.5z"/></svg></span><span class="nav-text">DeskTube</span><button class="collapse-button" type="button" onclick={() => (sidebarCollapsed = !sidebarCollapsed)} aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}>{#if sidebarCollapsed}<svg viewBox="0 0 32 32" class="collapse-mark"><path d="M12 8.5v15l11-7.5z"/></svg>{:else}<svg viewBox="0 0 24 24"><path d="m15 5-7 7 7 7"/></svg>{/if}</button></div>
    <nav aria-label="Main navigation">
      <button class:active={activeView === "home"} class="nav-item" type="button" onclick={() => void navigate("home")}>
        <svg viewBox="0 0 24 24"><path d="M3 11.5 12 4l9 7.5V21h-6v-6H9v6H3z" /></svg>
        <span class="nav-text">Home</span>
      </button>
      <button class:active={activeView === "explore"} class="nav-item" type="button" onclick={() => void navigate("explore")}>
        <svg viewBox="0 0 24 24"><path d="M12 3l2.2 5.7L20 11l-5.8 2.3L12 19l-2.2-5.7L4 11l5.8-2.3z"/></svg>
        <span class="nav-text">Explore</span>
      </button>
    </nav>
    <div class="nav-label">YOUR MUSIC</div>
    <nav>
      <button class:active={activeView === "liked"} class="nav-item" type="button" onclick={() => void navigate("liked")}>
        <svg viewBox="0 0 24 24"><path d="M12 21S4 16.5 4 9.5A4.5 4.5 0 0 1 12 6a4.5 4.5 0 0 1 8 3.5C20 16.5 12 21 12 21z"/></svg>
        <span class="nav-text">Liked songs</span>
      </button>
      <button class:active={activeView === "recent"} class="nav-item" type="button" onclick={() => void navigate("recent")}>
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/></svg>
        <span class="nav-text">Recently played</span>
      </button>
      <button class:active={activeView === "playlists" || activeView === "playlist"} class="nav-item" type="button" onclick={() => void navigate("playlists")}>
        <svg viewBox="0 0 24 24"><path d="M4 5h16M4 12h16M4 19h10"/></svg>
        <span class="nav-text">Playlists</span>
      </button>
      <button class="nav-item" type="button" onclick={() => void createNewPlaylist()} disabled={!auth.signed_in}>
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7v10M7 12h10"/></svg>
        <span class="nav-text">New playlist</span>
      </button>
    </nav>
    <div class="sidebar-account">
      {#if auth.signed_in}
        <button class="account-button" type="button" onclick={() => void signOut()} title="Sign out">
          {#if auth.profile?.avatar}<img src={auth.profile.avatar} alt="" onerror={coverFallback} />{:else}<span class="account-avatar"><svg viewBox="0 0 24 24"><path d="M12 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4Zm0 2c-4 0-7 2-7 5v1h14v-1c0-3-3-5-7-5Z"/></svg></span>{/if}
          <span class="nav-text"><strong>{auth.profile?.name}</strong><small>Sign out</small></span>
        </button>
      {:else}
        <button class="account-button signed-out" type="button" onclick={() => void signIn()} disabled={accountLoading || authPrompt !== null}>
          <span class="account-avatar"><svg viewBox="0 0 24 24"><path d="M12 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4Zm0 2c-4 0-7 2-7 5v1h14v-1c0-3-3-5-7-5Z"/></svg></span><span class="nav-text"><strong>{accountLoading ? "Connecting…" : "Connect Google"}</strong><small>Likes & playlists</small></span>
        </button>
        {#if authPrompt}
          <div class="device-code">
            {#if authPrompt.user_code}
              <span>Enter this code at google.com/device</span>
              <button type="button" onclick={() => void copyAuthCode()}>{authPrompt.user_code}</button>
              <small>{authCodeCopied ? "Copied" : "Click code to copy"}</small>
            {:else}
              <span>Finish signing in with Google in your browser.</span>
              <button type="button" onclick={() => void openUrl(authPrompt!.verification_url)}>Open browser again</button>
              <small>This window updates automatically after approval.</small>
            {/if}
          </div>
        {/if}
      {/if}
    </div>
  </aside>

  <section class="content">
    <header class="topbar">
      <form onsubmit={(event) => { event.preventDefault(); void search(); }}>
        <div class="search-box">
          <svg viewBox="0 0 24 24"><circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 5 5"/></svg>
          <input class="search-input" bind:value={query} placeholder="Search songs, artists, albums" aria-label="Search" aria-autocomplete="list" aria-expanded={searchFocused && suggestions.length > 0} onfocus={() => (searchFocused = true)} onblur={() => window.setTimeout(() => (searchFocused = false), 140)} onkeydown={handleSearchKey} />
          {#if query}<button class="clear" type="button" onclick={() => { query = ""; suggestions = []; }} aria-label="Clear search">×</button>{/if}
          {#if searchFocused && suggestions.length}
            <div class="suggestions" role="listbox" aria-label="Search suggestions">
              <div class="suggestions-label"><span>Suggestions</span><small>{suggestions.length} results</small></div>
              {#each suggestions as track, index}
                <button class:active={index === suggestionIndex} type="button" role="option" aria-selected={index === suggestionIndex} onmousedown={(event) => event.preventDefault()} onclick={() => void chooseSuggestion(track)} onmouseenter={() => (suggestionIndex = index)}>
                  {#if track.thumbnail}<img src={track.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="suggestion-cover">♪</span>{/if}
                  <span><strong>{track.title}</strong><small>{track.artist}</small></span>
                  <time>{track.duration}</time>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      </form>
      <div class="topbar-actions">
        <button class:active={settingsOpen} class="topbar-action" type="button" onclick={() => { settingsOpen = !settingsOpen; if (settingsOpen) remoteOpen = false; }} aria-label="Playback settings" title="Playback settings">
          <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3.2"/><path d="M12 2.8v2.6M12 18.6v2.6M4.9 4.9l1.9 1.9M17.2 17.2l1.9 1.9M2.8 12h2.6M18.6 12h2.6M4.9 19.1l1.9-1.9M17.2 6.8l1.9-1.9"/></svg>
        </button>
        <button class:active={remoteOpen} class="topbar-action" type="button" onclick={() => void toggleRemote()} aria-label="Phone remote" title="Control DeskTube from your phone">
          <svg viewBox="0 0 24 24"><rect x="7" y="2.5" width="10" height="19" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M11 18.5h2" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
        </button>
        {#if settingsOpen}
          <div class="settings-popover" role="dialog" aria-label="Playback settings">
            <div class="settings-row">
              <span>Sleep timer</span>
              <div class="sleep-options">
                {#each [15, 30, 45, 60] as minutes}
                  <button type="button" onclick={() => setSleepTimer(minutes)}>{minutes}m</button>
                {/each}
                <button type="button" onclick={() => setSleepTimer(0)}>Off</button>
              </div>
            </div>
            {#if sleepEndAt !== null}<small class="sleep-countdown">Sleeping in {formatTime(sleepRemainingMs)}</small>{/if}
            <label class="settings-row settings-toggle">
              <input type="checkbox" checked={crossfade} onchange={(event) => void setCrossfade(event.currentTarget.checked)} />
              <span>Crossfade between tracks</span>
            </label>
            <div class="settings-row">
              <span>Output device</span>
              <select value={audioDevice} onchange={(event) => void pickAudioDevice(event.currentTarget.value)} aria-label="Output device">
                <option value="">System default</option>
                {#each audioDevices as device}<option value={device}>{device}</option>{/each}
              </select>
            </div>
          </div>
        {/if}
        {#if remoteOpen}
          <div class="settings-popover remote-popover" role="dialog" aria-label="Phone remote">
            <div class="settings-row"><span>Phone remote</span></div>
            {#if remoteInfo?.qr_svg}
              <div class="remote-qr">{@html remoteInfo.qr_svg}</div>
              <small class="remote-hint">Scan with your phone camera — same Wi-Fi required.</small>
              <small class="remote-url">{remoteInfo.url.replace(/\?t=.*/, "?t=…")}</small>
            {:else}
              <small class="remote-hint">Remote unavailable — check that Windows Firewall allows DeskTube on your local network.</small>
            {/if}
          </div>
        {/if}
      </div>
    </header>

    <div class="stage">
      <div class="stage-heading">
        <div>
          <span class="eyebrow">{activeView === "home" ? "GOOD TO SEE YOU" : activeView === "search" ? "DISCOVER" : "YOUR MUSIC"}</span>
          <h1>{pageTitle}</h1>
          <p>{pageSubtitle}</p>
        </div>
        <div class="heading-actions">
          {#if activeView === "explore"}<button class="play-all explore-refresh" type="button" onclick={() => void buildExplore()} disabled={exploreLoading} title="New recommendations"><svg viewBox="0 0 24 24"><path d="M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5"/></svg>{exploreLoading ? "Mixing…" : "Refresh"}</button>{/if}
          {#if visibleTracks.length}<button class="play-all" type="button" onclick={() => { originalQueue = [...visibleTracks]; queue = shuffle(visibleTracks); shuffleMode = true; localStorage.setItem("desktube-shuffle", "1"); void playAt(0); }}>Shuffle music</button>{/if}
        </div>
      </div>

      {#if error}<div class="error"><strong>DeskTube</strong><span>{error}</span></div>{/if}

      {#if accountLoading || searching || exploreLoading}
        <div class="loading-list">{#each Array(8) as _}<div class="skeleton"></div>{/each}</div>
      {:else if activeView === "home"}
        {#if heroTrack}
          <section class="editorial-hero">
            <div class="hero-copy"><span>MADE FOR {auth.profile?.name?.toUpperCase() ?? "YOU"}</span><h2>{heroTrack.title}</h2><p>{heroTrack.artist}</p><div><button class="hero-play" type="button" onclick={() => void playHomeTrack(heroTrack)}><svg viewBox="0 0 24 24"><path d="m8 5 11 7-11 7z"/></svg>Play</button><button class="hero-library" type="button" onclick={() => void navigate("liked")}>Open library</button></div></div>
            {#if heroTrack.thumbnail}<img src={highQualityCover(heroTrack)} alt="" onerror={(event) => expandedCoverFallback(event, heroTrack)} />{:else}<div class="hero-fallback">♪</div>{/if}
          </section>
        {:else}
          <section class="editorial-hero welcome-hero"><div class="hero-copy"><span>DESKTUBE</span><h2>Music, without the noise.</h2><p>A focused native player shaped around your listening.</p><div><button class="hero-play" type="button" onclick={() => void navigate("search")}>Start listening</button>{#if !auth.signed_in}<button class="hero-library" type="button" onclick={() => void signIn()}>Connect Google</button>{/if}</div></div><div class="hero-orbit"><i></i><i></i><i></i></div></section>
        {/if}
        <div class="mood-row">
          {#each MOODS as mood}
            <button class:active={activeMood === mood.label} class="mood-chip" type="button" style={`--chip-color:${mood.color}`} onclick={() => void selectMood(mood)}>{mood.label}</button>
          {/each}
        </div>
        {#if moodLoading}
          <div class="quick-grid">{#each Array(6) as _}<div class="skeleton tile"></div>{/each}</div>
        {:else if gridTracks.length}
          <section class="editorial-section"><div class="section-title"><div><span>{activeMood ? "MOOD MIX" : "QUICK PICKS"}</span><h2>{activeMood ?? "Back into your music"}</h2></div><button type="button" onclick={() => (activeMood ? openMoodSearch() : void navigate("liked"))}>See all</button></div><div class="quick-grid">{#each gridTracks as track}<button type="button" onclick={() => void playGridTrack(track)}>{#if track.thumbnail}<img src={track.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="mini-cover">♪</span>{/if}<strong>{track.title}</strong><svg viewBox="0 0 24 24"><path d="m8 5 11 7-11 7z"/></svg></button>{/each}</div></section>
        {/if}
        {#if !activeMood && recentTracks.length}
          <section class="editorial-section"><div class="section-title"><div><span>RECENTLY PLAYED</span><h2>Jump back in</h2></div><button type="button" onclick={() => void navigate("recent")}>See all</button></div><div class="quick-grid">{#each recentTracks.slice(0, 6) as track}<button type="button" onclick={() => { queue = [...recentTracks]; void playAt(queue.findIndex((item) => item.video_id === track.video_id)); }}>{#if track.thumbnail}<img src={track.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="mini-cover">♪</span>{/if}<strong>{track.title}</strong><svg viewBox="0 0 24 24"><path d="m8 5 11 7-11 7z"/></svg></button>{/each}</div></section>
        {/if}
        {#if playlists.length}
          <section class="editorial-section"><div class="section-title"><div><span>YOUR COLLECTION</span><h2>Playlists</h2></div><button type="button" onclick={() => void navigate("playlists")}>See all</button></div><div class="editorial-shelf">{#each playlists.slice(0, 6) as playlist}<button type="button" onclick={() => void openPlaylist(playlist)}>{#if playlist.thumbnail}<img src={playlist.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="shelf-cover">♪</span>{/if}<strong>{playlist.title}</strong><small>{playlist.item_count} tracks</small></button>{/each}</div></section>
        {/if}
      {:else if !auth.signed_in && (activeView === "liked" || activeView === "playlists")}
        <div class="connect-card"><span class="account-avatar">G</span><h2>Connect your Google account</h2><p>Sign in to load your YouTube Music likes and playlists.</p><button type="button" onclick={() => void signIn()}>Continue with Google</button></div>
      {:else if activeView === "playlists"}
        <div class="playlist-grid">
          {#each playlists as playlist}
            <button class="playlist-card" type="button" onclick={() => void openPlaylist(playlist)}>
              {#if playlist.thumbnail}<img src={playlist.thumbnail} alt="" onerror={coverFallback} />{:else}<div class="playlist-fallback">♪</div>{/if}
              <strong>{playlist.title}</strong><span>{playlist.item_count} items</span>
            </button>
          {/each}
          {#if !playlists.length}<div class="empty-library">No playlists found on this account.</div>{/if}
        </div>
      {:else if visibleTracks.length || activeView === "artist" || activeView === "album"}
        {#if activeView === "artist" || activeView === "album"}
          <button class="back-button" type="button" onclick={goBack}><svg viewBox="0 0 24 24"><path d="M15 5l-7 7 7 7"/></svg>Back</button>
        {/if}
        {#if activeView === "artist" && artistPage}
          <section class="entity-hero">
            {#if artistPage.thumbnail}<img src={artistPage.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="entity-fallback"><svg viewBox="0 0 24 24"><path d="M12 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4Zm0 2c-4 0-7 2-7 5v1h14v-1c0-3-3-5-7-5Z"/></svg></span>{/if}
            <div><span class="eyebrow">ARTIST</span><h2>{artistPage.name}</h2></div>
          </section>
          {#if artistPage.albums.length}
            <section class="editorial-section"><div class="section-title"><div><span>DISCOGRAPHY</span><h2>Albums & singles</h2></div></div><div class="editorial-shelf">{#each artistPage.albums as item}<button type="button" onclick={() => void openAlbum(item.id)}>{#if item.thumbnail}<img src={item.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="shelf-cover">♪</span>{/if}<strong>{item.title}</strong><small>{item.subtitle}</small></button>{/each}</div></section>
          {/if}
          {#if artistPage.top_songs.length}<h3 class="shelf-title">Top songs</h3>{/if}
        {:else if activeView === "album" && albumPage}
          <section class="entity-hero">
            {#if albumPage.thumbnail}<img src={albumPage.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="entity-fallback">♪</span>{/if}
            <div><span class="eyebrow">ALBUM</span><h2>{albumPage.title}</h2><p>{albumPage.artist}</p></div>
          </section>
        {/if}
        {#if visibleTracks.length}
        <div class="track-table">
          <div class="table-head"><span>#</span><span>Title</span><span>Artist</span><span></span><span>Time</span></div>
          {#each visibleTracks as track, index}
            <div class:playing={currentTrack?.video_id === track.video_id} class="track-row">
              <button class="track-hit" type="button" onclick={() => void playResult(track)} aria-label={`Play ${track.title}`}>
                <span class="track-number"><span class="number">{index + 1}</span><svg class="row-play" viewBox="0 0 24 24"><path d="m8 5 11 7-11 7z"/></svg></span>
                <span class="track-title-cell">
                  {#if track.thumbnail}<img src={track.thumbnail} alt="" onerror={coverFallback} />{:else}<span class="cover-fallback">♪</span>{/if}
                  <span><strong>{track.title}</strong><small>{track.album || "Song"}</small></span>
                </span>
              </button>
              <button class="track-artist" type="button" onclick={() => void openArtist(track)} aria-label={`Open artist ${track.artist}`}>{track.artist}</button>
              <button class="queue-add" type="button" onclick={() => playNext(track)} disabled={currentTrack?.video_id === track.video_id} aria-label={`Play ${track.title} next`} title="Play next">
                <svg viewBox="0 0 24 24"><path d="M4 6h11M4 12h11M4 18h11M18 8v8l5-4z"/></svg>
              </button>
              <span class="track-duration">{track.duration}</span>
            </div>
          {/each}
        </div>
        {/if}
      {:else}
        <div class="empty-state">
          <div class="empty-art"><span></span><span></span><span></span><span></span></div>
          <h2>{activeView === "liked" ? "No liked music found" : activeView === "playlist" ? "This playlist is empty" : activeView === "recent" ? "Nothing played yet" : activeView === "explore" ? "No recommendations yet" : "Your music starts here"}</h2>
          <p>{activeView === "explore" ? "Play or like some songs — DeskTube builds your mix from your taste." : activeView === "search" ? "Try a different song, artist, or album." : "Try another section or search for a song."}</p>
        </div>
      {/if}
    </div>
  </section>

  {#if nowPlayingExpanded && currentTrack}
    <section class="now-playing-expanded" aria-label="Expanded now playing" style={npAccent ? `--np-accent:${npAccent}` : ""}>
      <button class="collapse-now-playing" type="button" onclick={() => (nowPlayingExpanded = false)} aria-label="Collapse now playing"><svg viewBox="0 0 24 24"><path d="m5 9 7 7 7-7"/></svg></button>
      <button class="focus-toggle" type="button" onclick={() => void setFocusMode(!focusMode)} aria-label={focusMode ? "Exit focus mode (Esc)" : "Enter focus mode"} title={focusMode ? "Exit focus — Esc" : "Focus mode"}>
        <svg viewBox="0 0 24 24">{#if focusMode}<path d="M9 15l-5 5m0 0v-4m0 4h4M15 9l5-5m0 0v4m0-4h-4"/>{:else}<path d="M9 3 4 8m0 0V4m0 4h4M15 21l5-5m0 0v4m0-4h-4"/>{/if}</svg>
      </button>
      <div class="expanded-artwork">
        {#key currentTrack.video_id}
          <img src={highQualityCover(currentTrack)} alt={`${currentTrack.title} cover`} onerror={(event) => expandedCoverFallback(event, currentTrack)} />
        {/key}
        <div class="expanded-meta"><span>NOW PLAYING</span><h2>{currentTrack.title}</h2><p><button class="meta-link" type="button" onclick={() => void openArtist(currentTrack)}>{currentTrack.artist}</button>{#if currentTrack.album && currentTrack.album_id} · <button class="meta-link" type="button" onclick={() => { if (currentTrack.album_id) void openAlbum(currentTrack.album_id); }}>{currentTrack.album}</button>{/if}</p></div>
      </div>
      <div class="lyrics-panel">
        <div class="lyrics-heading"><span>LYRICS</span>{#if lyricLines.some((line) => line.time_ms !== null)}<small>SYNCED</small>{/if}</div>
        <div class="lyrics-scroll">
          {#if lyricsLoading}
            <div class="lyrics-loading"><span></span><span></span><span></span></div>
          {:else if lyricsMessage}
            <div class="lyrics-message"><span>{lyricsMessage}</span><button type="button" onclick={() => void loadLyrics(currentTrack)}>Try again</button></div>
          {:else}
            {#each lyricLines as line, index}
              <button id={`lyric-${index}`} class:active={index === activeLyricIndex} class:past={line.time_ms !== null && index < activeLyricIndex} type="button" disabled={line.time_ms === null} onclick={() => line.time_ms !== null && void seekTo(line.time_ms)}>{line.text}</button>
            {/each}
          {/if}
        </div>
      </div>
    </section>
  {/if}

  <footer class="player-dock">
    {#if currentTrack}<button class="dock-expand" type="button" onclick={() => void toggleNowPlaying()} aria-label={nowPlayingExpanded ? "Collapse now playing" : "Expand now playing"}></button>{/if}
    <div class="dock-left">
      <button class="dock-track" type="button" onclick={() => void toggleNowPlaying()} disabled={!currentTrack} aria-label="Expand now playing">
        {#if currentTrack}
          {#if currentTrack.thumbnail}<img src={currentTrack.thumbnail} alt="" onerror={coverFallback} />{:else}<div class="dock-cover app-mark"><svg viewBox="0 0 32 32" aria-hidden="true"><path d="M12 8.5v15l11-7.5z"/></svg></div>{/if}
          <div><strong>{currentTrack.title}</strong><span>{currentTrack.artist}</span></div>
        {:else}
          <div class="dock-cover app-mark idle"><svg viewBox="0 0 32 32" aria-hidden="true"><path d="M12 8.5v15l11-7.5z"/></svg></div><div><strong>Nothing playing</strong><span>Choose a song</span></div>
        {/if}
      </button>
    </div>

    <div class="transport">
      <div class="transport-buttons">
        <button class:active={shuffleMode} class="shuffle-toggle" type="button" onclick={toggleShuffle} disabled={queue.length < 2} aria-label={shuffleMode ? "Shuffle on" : "Shuffle off"} title={shuffleMode ? "Shuffle on" : "Shuffle off"}>
          <svg viewBox="0 0 24 24"><path d="M3 6h4l10 12h4m0 0-2.5-2.5M21 18l-2.5 2.5M3 18h4l2.5-3M21 6h-4l-3.5 4.5M21 6l-2.5-2.5M21 6l-2.5 2.5"/></svg>
        </button>
        <button type="button" onclick={() => void previousTrack()} disabled={!currentTrack} aria-label="Previous track">
          <svg viewBox="0 0 24 24"><path d="M6 5v14M19 6 9 12l10 6z"/></svg>
        </button>
        <button class="seek-ten" type="button" onclick={() => void seekBy(-10000)} disabled={!currentTrack || loadingTrack} aria-label="Back 10 seconds"><svg viewBox="0 0 24 24"><path d="M5 8V3m0 0h5M5 3l3 3M6 17a8 8 0 1 0 1-10"/></svg><span>10</span></button>
        <button class="main-play" type="button" onclick={() => void togglePause()} disabled={!currentTrack || loadingTrack} aria-label={playback.status === "playing" ? "Pause" : "Play"}>
          {#if loadingTrack}<span class="spinner"></span>{:else if playback.status === "playing"}<svg viewBox="0 0 24 24"><path d="M7 5h4v14H7zM14 5h4v14h-4z"/></svg>{:else}<svg viewBox="0 0 24 24"><path d="m8 5 11 7-11 7z"/></svg>{/if}
        </button>
        <button class="seek-ten" type="button" onclick={() => void seekBy(10000)} disabled={!currentTrack || loadingTrack} aria-label="Forward 10 seconds"><svg viewBox="0 0 24 24"><path d="M19 8V3m0 0h-5m5 0-3 3m2 11a8 8 0 1 1-1-10"/></svg><span>10</span></button>
        <button type="button" onclick={() => void nextTrack()} disabled={!currentTrack || currentIndex >= queue.length - 1} aria-label="Next track">
          <svg viewBox="0 0 24 24"><path d="M18 5v14M5 6l10 6-10 6z"/></svg>
        </button>
      </div>
      <div class="timeline">
        <time>{formatTime(scrubPosition)}</time>
        <input class="scrubber" type="range" min="0" max={timelineDuration || 1} step="250" value={scrubPosition} disabled={!currentTrack || loadingTrack} onpointerdown={() => (scrubbing = true)} oninput={(event) => { scrubbing = true; scrubPosition = Number(event.currentTarget.value); }} onchange={(event) => void seekTo(Number(event.currentTarget.value))} aria-label="Playback position" style={`--progress:${timelineDuration ? (scrubPosition / timelineDuration) * 100 : 0}%`} />
        <time>{formatTime(timelineDuration)}</time>
      </div>
    </div>

    <div class="volume">
      {#if currentTrack}
        <button class:liked={likedIds.has(currentTrack.video_id)} class="dock-like" type="button" onclick={() => void toggleLike(currentTrack)} aria-label={likedIds.has(currentTrack.video_id) ? "Unlike this song" : "Like this song"} title={auth.signed_in ? (likedIds.has(currentTrack.video_id) ? "Unlike" : "Like") : "Connect Google to like"}>
          <svg viewBox="0 0 24 24"><path d="M12 21S4 16.5 4 9.5A4.5 4.5 0 0 1 12 6a4.5 4.5 0 0 1 8 3.5C20 16.5 12 21 12 21z"/></svg>
        </button>
      {/if}
      {#if currentTrack}
        <button class:active={radioMode} class="dock-radio" type="button" onclick={() => void toggleRadio()} aria-label="Toggle radio — endless mix from this song" title={radioMode ? "Radio on — tap to turn off" : "Start radio — endless similar songs"}>
          <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="2"/><path d="M7.5 7.5a6.4 6.4 0 0 0 0 9M16.5 7.5a6.4 6.4 0 0 1 0 9M4.6 4.6a10.5 10.5 0 0 0 0 14.8M19.4 4.6a10.5 10.5 0 0 1 0 14.8"/></svg>
        </button>
      {/if}
      <button class="volume-toggle" type="button" onclick={() => void toggleMute()} aria-label={muted ? "Unmute" : "Mute"}>
        {#if muted}
          <svg viewBox="0 0 24 24"><path d="M4 10v4h4l5 4V6l-5 4zM17 9l5 6m0-6-5 6"/></svg>
        {:else if volume === 0}
          <svg viewBox="0 0 24 24"><path d="M4 10v4h4l5 4V6l-5 4z"/></svg>
        {:else if volume < 1}
          <svg viewBox="0 0 24 24"><path d="M4 10v4h4l5 4V6l-5 4zM17 9a4 4 0 0 1 0 6"/></svg>
        {:else}
          <svg viewBox="0 0 24 24"><path d="M4 10v4h4l5 4V6l-5 4zM17 9a4 4 0 0 1 0 6M19 6a8 8 0 0 1 0 12"/></svg>
        {/if}
      </button>
      <input type="range" min="0" max="2" step="0.01" value={muted ? 0 : volume} oninput={setVolume} aria-label="Volume" style={`--volume:${(muted ? 0 : volume) * 50}%`} />
      <span>{Math.round((muted ? 0 : volume) * 100)}</span>
    </div>
  </footer>
</div>

<style>
  :global(*) { box-sizing: border-box; }
  :global(body) { margin: 0; min-width: 820px; background: #000; color: #fff; font-family: "Segoe UI Variable", "Segoe UI", system-ui, sans-serif; overflow: hidden; }
  :global(button), :global(input) { font: inherit; }
  :global(button) { color: inherit; }
  .app-shell { --accent: #1ed760; display: grid; grid-template-columns: 220px minmax(430px, 1fr); grid-template-rows: minmax(0, 1fr) 88px; gap: 8px; height: 100vh; padding: 8px 8px 0; background: #000; transition: grid-template-columns .2s ease; }
  .app-shell.sidebar-collapsed { grid-template-columns: 72px minmax(430px, 1fr); }
  svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .sidebar { grid-row: 1; padding: 22px 14px; overflow: hidden; background: #121212; border-radius: 9px; display: flex; flex-direction: column; }
  .brand { display: flex; align-items: center; gap: 10px; padding: 0 8px 26px; font-weight: 750; font-size: 19px; letter-spacing: -.4px; white-space: nowrap; }
  .brand-mark { display: grid; place-items: center; width: 32px; height: 32px; flex: 0 0 32px; border-radius: 50%; background: linear-gradient(145deg, #35ed7a, #1db954); box-shadow: 0 7px 26px #1ed76035, inset 0 1px #ffffff45; }
  .brand-mark svg { width: 21px; height: 21px; fill: #fff; stroke: none; }
  .collapse-button { display: grid; place-items: center; width: 28px; height: 28px; margin-left: auto; padding: 0; border: 0; border-radius: 50%; background: #242424; color: #a7a7a7; cursor: pointer; }
  .collapse-button:hover { color: #fff; background: #303030; }
  .collapse-button svg { width: 15px; }
  .sidebar-collapsed .sidebar { padding-inline: 8px; }
  .sidebar-collapsed .brand { justify-content: center; padding-inline: 0; }
  .sidebar-collapsed .brand-mark, .sidebar-collapsed .nav-text, .sidebar-collapsed .nav-label { display: none; }
  .sidebar-collapsed .collapse-button { width: 36px; height: 36px; margin: 0; background: var(--accent); color: #fff; box-shadow: 0 4px 14px #1ed76040; }
  .sidebar-collapsed .collapse-button:hover { background: #22e768; }
  .collapse-mark { width: 19px; height: 19px; fill: currentColor; stroke: none; }
  .sidebar-collapsed .nav-item { justify-content: center; padding-inline: 0; }
  .sidebar-collapsed .sidebar-account { padding-inline: 0; }
  .sidebar-collapsed .account-button { justify-content: center; width: 44px; margin-inline: auto; padding-inline: 0; }
  .sidebar-collapsed .account-button img, .sidebar-collapsed .account-avatar { width: 32px; height: 32px; flex-basis: 32px; }
  nav { display: grid; gap: 4px; }
  .nav-item { display: flex; align-items: center; gap: 12px; width: 100%; padding: 10px 11px; border: 0; border-radius: 8px; background: transparent; color: #8d9099; font-size: 13px; font-weight: 600; text-align: left; cursor: pointer; }
  .nav-item:hover, .nav-item.active { background: #282828; color: #fff; }
  .nav-item.active { box-shadow: none; }
  .nav-item svg { width: 18px; }
  .nav-item.active svg { color: var(--accent); }
  .nav-label { margin: 30px 11px 10px; color: #575a63; font-size: 10px; font-weight: 800; letter-spacing: 1.4px; }
  .sidebar-account { margin-top: auto; padding: 8px 2px 0; border-top: 1px solid #ffffff0b; }
  .account-button { display: flex; align-items: center; gap: 9px; width: 100%; min-width: 0; padding: 9px; border: 0; border-radius: 9px; background: transparent; text-align: left; cursor: pointer; }
  .account-button:hover { background: #282828; }
  .account-button .nav-text { min-width: 0; display: grid; gap: 2px; }
  .sidebar-collapsed .account-button .nav-text { display: none; }
  .account-button strong, .account-button small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .account-button strong { font-size: 11px; }.account-button small { color: #777; font-size: 9px; }
  .account-button img, .account-avatar { display: grid; place-items: center; width: 30px; height: 30px; flex: 0 0 30px; border-radius: 50%; object-fit: cover; background: #1ed760; color: #07150b; font-size: 12px; font-weight: 900; }
  .account-avatar svg { width: 16px; height: 16px; fill: currentColor; stroke: none; }
  .device-code { display: grid; gap: 6px; margin-top: 8px; padding: 10px; border: 1px solid #1ed76044; border-radius: 10px; background: #1ed76012; }
  .device-code span, .device-code small { color: #9a9da5; font-size: 9px; line-height: 1.35; text-align: center; }
  .device-code button { padding: 4px; border: 0; border-radius: 6px; background: transparent; color: var(--accent); font-size: 16px; font-weight: 800; letter-spacing: 1.4px; cursor: copy; }
  .device-code button:hover { background: #1ed76016; }
  .sidebar-collapsed .device-code { display: none; }
  .content { min-width: 0; overflow: hidden; display: flex; flex-direction: column; border-radius: 9px; background: linear-gradient(180deg, #242424 0, #121212 190px); }
  .topbar { display: flex; align-items: center; gap: 14px; padding: 16px 26px; background: rgba(18,18,18,.76); backdrop-filter: blur(18px); }
  .topbar form { flex: 1; min-width: 0; }
  .topbar-actions { position: relative; display: flex; align-items: center; gap: 6px; color: #8d9099; }
  .topbar-action { display: grid; place-items: center; padding: 7px; border: 0; border-radius: 8px; background: transparent; color: inherit; cursor: pointer; transition: color .15s, background .15s; }
  .topbar-action:hover { color: #fff; background: #ffffff14; }
  .topbar-action.active { color: var(--accent); }
  .topbar-action svg { width: 18px; height: 18px; }
  .transport .shuffle-toggle { position: relative; width: 32px; height: 32px; border: 1px solid #ffffff20; border-radius: 8px; background: #ffffff0a; color: #d5d7dd; transition: color .15s, background .15s; }
  .transport .shuffle-toggle:hover { color: #fff; background: #ffffff20; }
  .transport .shuffle-toggle.active { color: var(--accent); border-color: #1ed76066; background: #1ed76020; }
  .transport .shuffle-toggle.active::after { content: ""; position: absolute; bottom: 2px; width: 3px; height: 3px; border-radius: 50%; background: var(--accent); }
  .transport .shuffle-toggle:disabled { opacity: .4; cursor: default; }
  .transport .shuffle-toggle svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  form { display: flex; max-width: 760px; gap: 10px; }
  .search-box { position: relative; flex: 1; }
  .search-box > svg { position: absolute; left: 14px; top: 50%; translate: 0 -50%; width: 17px; color: var(--accent); }
  .search-input { width: 100%; height: 44px; padding: 0 44px; border: 1px solid transparent; border-radius: 24px; outline: 0; background: #2a2a2a; color: #fff; transition: .2s; }
  .search-input:hover { background: #303030; }
  .search-input:focus { border-color: #fff; box-shadow: none; background: #2a2a2a; }
  .clear { position: absolute; right: 7px; top: 6px; width: 30px; height: 30px; border: 0; background: transparent; color: #777; font-size: 20px; cursor: pointer; }
  .suggestions { position: absolute; z-index: 20; top: 52px; left: 0; right: 0; max-height: min(590px, calc(100vh - 180px)); padding: 8px; overflow-y: auto; border: 1px solid #ffffff14; border-radius: 16px; background: #181918f2; box-shadow: 0 24px 65px #000c, inset 0 1px #ffffff12; backdrop-filter: blur(24px) saturate(1.2); scrollbar-width: thin; }
  .suggestions-label { display: flex; justify-content: space-between; padding: 7px 9px 9px; color: #8a8a8a; font-size: 9px; font-weight: 800; letter-spacing: 1.2px; text-transform: uppercase; }
  .suggestions-label small { font-size: 8px; font-weight: 700; letter-spacing: .4px; }
  .suggestions > button { display: grid; grid-template-columns: 42px minmax(0, 1fr) auto; align-items: center; gap: 11px; width: 100%; min-width: 0; padding: 7px 9px; border: 0; border-radius: 10px; background: transparent; text-align: left; cursor: pointer; }
  .suggestions > button:hover, .suggestions > button.active { background: #ffffff12; }
  .suggestions img, .suggestion-cover { display: grid; place-items: center; width: 42px; height: 42px; border-radius: 7px; object-fit: cover; background: linear-gradient(145deg, #245832, #171817); color: var(--accent); }
  .suggestions > button > span:nth-child(2) { min-width: 0; display: grid; gap: 3px; }
  .suggestions strong, .suggestions small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.suggestions strong { font-size: 11px; }.suggestions small, .suggestions time { color: #828282; font-size: 9px; }
  .play-all { border: 0; border-radius: 999px; padding: 0 22px; background: var(--accent); color: #000; font-weight: 800; font-size: 12px; cursor: pointer; box-shadow: none; }
  .play-all:hover { scale: 1.03; background: #3be477; }
  button:disabled { opacity: .38; cursor: default; }
  .stage { overflow-y: auto; padding: 28px 30px 50px; scrollbar-color: #34363d transparent; }
  .stage-heading { display: flex; justify-content: space-between; align-items: end; margin-bottom: 24px; }
  .eyebrow { color: var(--accent); font-size: 9px; font-weight: 900; letter-spacing: 1.6px; }
  h1 { margin: 5px 0 5px; font-size: 30px; font-weight: 800; letter-spacing: -1.2px; }
  .stage-heading p { margin: 0; color: #a7a7a7; font-size: 12px; }
  .play-all { height: 36px; }
  .error { display: flex; gap: 10px; margin: 0 0 18px; padding: 11px 13px; border: 1px solid #682a2d; border-radius: 9px; background: #291517; color: #ffaaa8; font-size: 11px; }
  .error span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track-table { min-width: 520px; }
  .table-head, .track-row { display: grid; grid-template-columns: 36px minmax(190px, 1.3fr) minmax(120px, .8fr) 30px 48px; align-items: center; gap: 10px; }
  .heading-actions { display: flex; align-items: center; gap: 10px; }
  .explore-refresh { display: inline-flex; align-items: center; gap: 7px; background: transparent; border: 1px solid #ffffff22; color: #c8cad0; }
  .explore-refresh:hover { border-color: var(--accent); color: #fff; }
  .explore-refresh svg { width: 14px; height: 14px; }
  .table-head { padding: 0 12px 8px; color: #5f626b; font-size: 9px; font-weight: 800; letter-spacing: 1px; text-transform: uppercase; border-bottom: 1px solid #202126; }
  .track-row { width: 100%; min-height: 58px; padding: 7px 12px; border: 0; border-bottom: 1px solid #17181c; border-radius: 7px; background: transparent; text-align: left; cursor: pointer; }
  .queue-add { display: grid; place-items: center; padding: 5px; border: 0; border-radius: 6px; background: transparent; color: #5c606b; cursor: pointer; opacity: 0; transition: opacity .15s, color .15s, background .15s; }
  .track-row:hover .queue-add { opacity: 1; }
  .queue-add:hover { color: var(--accent); background: #ffffff14; }
  .queue-add:disabled { opacity: 0; }
  .queue-add svg { width: 15px; height: 15px; }
  .track-row:hover { background: #2a2a2a; }
  .track-row.playing { background: #242424; }
  .track-row.playing strong, .track-row.playing .track-number { color: var(--accent); }
  .track-number { color: #656871; font-size: 11px; text-align: center; }
  .row-play { display: none; width: 15px; margin: auto; fill: currentColor; stroke: none; }
  .track-row:hover .number { display: none; }
  .track-row:hover .row-play { display: block; color: #fff; }
  .track-title-cell { display: flex; align-items: center; min-width: 0; gap: 11px; }
  .track-title-cell img, .cover-fallback { width: 42px; height: 42px; flex: 0 0 42px; border-radius: 6px; object-fit: cover; }
  .cover-fallback, .dock-cover { display: grid; place-items: center; background: linear-gradient(145deg, #282828, #181818); color: #1ed760; }
  .track-title-cell > span:last-child { min-width: 0; display: grid; gap: 3px; }
  .track-title-cell strong { overflow: hidden; color: #e8e9ec; font-size: 12px; font-weight: 650; text-overflow: ellipsis; white-space: nowrap; }
  .track-title-cell small { color: #53565e; font-size: 9px; }
  .track-artist, .track-duration { overflow: hidden; color: #7d8089; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
  .track-duration { text-align: right; }
  .track-hit { display: grid; grid-template-columns: 36px minmax(0, 1fr); align-items: center; gap: 10px; grid-column: 1 / 3; min-width: 0; padding: 0; border: 0; background: transparent; color: inherit; text-align: left; cursor: pointer; }
  button.track-artist { padding: 0; border: 0; background: transparent; text-align: left; cursor: pointer; }
  button.track-artist:hover { color: #fff; text-decoration: underline; }
  .loading-list { display: grid; gap: 8px; }
  .skeleton { height: 55px; border-radius: 8px; background: linear-gradient(90deg, #17181c 25%, #202126 45%, #17181c 65%); background-size: 300% 100%; animation: shimmer 1.4s infinite; }
  @keyframes shimmer { to { background-position: -150% 0; } }
  .empty-state { min-height: 55vh; display: grid; place-items: center; align-content: center; text-align: center; }
  .empty-state h2 { margin: 22px 0 5px; font-size: 18px; }
  .empty-state p { margin: 0; color: #666972; font-size: 12px; }
  .empty-art { display: flex; align-items: end; gap: 5px; width: 90px; height: 90px; padding: 23px; border: 1px solid #292b31; border-radius: 50%; background: #15161a; }
  .empty-art span { width: 7px; border-radius: 4px; background: linear-gradient(#1ed760, #117a37); animation: bars 1.2s ease-in-out infinite alternate; }
  .empty-art span:nth-child(1) { height: 18px; }.empty-art span:nth-child(2) { height: 38px; animation-delay: -.4s; }.empty-art span:nth-child(3) { height: 29px; animation-delay: -.7s; }.empty-art span:nth-child(4) { height: 14px; animation-delay: -.2s; }
  @keyframes bars { to { height: 10px; } }
  .connect-card { min-height: 48vh; display: grid; place-items: center; align-content: center; gap: 10px; text-align: center; }
  .connect-card .account-avatar { width: 52px; height: 52px; font-size: 20px; }
  .connect-card h2 { margin: 7px 0 0; }.connect-card p { margin: 0; color: #a7a7a7; font-size: 12px; }
  .connect-card button { margin-top: 8px; padding: 11px 20px; border: 0; border-radius: 999px; background: #fff; color: #000; font-size: 12px; font-weight: 800; cursor: pointer; }
  .playlist-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 16px; }
  .playlist-card { min-width: 0; padding: 13px; border: 1px solid #ffffff08; border-radius: 10px; background: #181818; text-align: left; cursor: pointer; transition: background .18s, transform .18s; }
  .playlist-card:hover { background: #282828; transform: translateY(-3px); }
  .playlist-card img, .playlist-fallback { width: 100%; aspect-ratio: 1; margin-bottom: 12px; border-radius: 7px; object-fit: cover; box-shadow: 0 10px 25px #0008; }
  .playlist-fallback { display: grid; place-items: center; background: linear-gradient(145deg, #294532, #181818); color: var(--accent); font-size: 34px; }
  .playlist-card strong, .playlist-card span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .playlist-card strong { font-size: 12px; }.playlist-card span { margin-top: 5px; color: #8b8b8b; font-size: 10px; }
  .empty-library { color: #8b8b8b; font-size: 12px; }
  .editorial-hero { position: relative; display: grid; grid-template-columns: minmax(0, 1fr) minmax(220px, 38%); min-height: 300px; margin-bottom: 42px; overflow: hidden; border: 1px solid #ffffff10; border-radius: 24px; background: linear-gradient(120deg, #284332 0%, #17241b 48%, #111 100%); box-shadow: inset 0 1px #ffffff18, 0 28px 70px #0006; }
  .editorial-hero::after { content: ""; position: absolute; inset: 0; pointer-events: none; background: radial-gradient(circle at 80% 10%, #ffffff17, transparent 34%); }
  .editorial-hero > img, .hero-fallback { width: 100%; height: 100%; min-height: 300px; object-fit: cover; mask-image: linear-gradient(90deg, transparent 0, #000 28%); }
  .hero-fallback { display: grid; place-items: center; background: linear-gradient(145deg, #1ed760, #0c4925); color: #07150b; font-size: 70px; }
  .hero-copy { z-index: 1; align-self: end; padding: clamp(30px, 5vw, 58px); }
  .hero-copy > span, .section-title span { color: #b7d8c1; font-size: 9px; font-weight: 900; letter-spacing: 1.8px; }
  .hero-copy h2 { max-width: 620px; margin: 9px 0 5px; font-size: clamp(34px, 5vw, 64px); font-weight: 780; line-height: .98; letter-spacing: -2.7px; }
  .hero-copy p { margin: 0 0 24px; color: #c1c7c3; font-size: 14px; }
  .hero-copy > div { display: flex; gap: 10px; }
  .hero-play, .hero-library { display: flex; align-items: center; gap: 7px; padding: 11px 20px; border: 0; border-radius: 999px; font-size: 12px; font-weight: 800; cursor: pointer; }
  .hero-play { background: var(--accent); color: #06130a; box-shadow: 0 9px 28px #1ed76030; }.hero-play svg { width: 15px; fill: currentColor; stroke: none; }
  .hero-library { background: #ffffff12; color: #fff; backdrop-filter: blur(14px); box-shadow: inset 0 0 0 1px #ffffff22; }
  .hero-play:hover, .hero-library:hover { transform: scale(1.025); }
  .welcome-hero { grid-template-columns: 1fr 260px; background: linear-gradient(120deg, #171918, #101110); }
  .hero-orbit { position: relative; margin: 40px; }
  .hero-orbit i { position: absolute; inset: 12%; border: 1px solid #1ed76045; border-radius: 50%; animation: orbit 12s linear infinite; }.hero-orbit i:nth-child(2) { inset: 27%; animation-direction: reverse; animation-duration: 8s; }.hero-orbit i:nth-child(3) { inset: 42%; background: var(--accent); box-shadow: 0 0 60px #1ed76070; }
  @keyframes orbit { to { transform: rotate(360deg); } }
  .mood-row { display: flex; gap: 12px; margin: 28px 0 24px; overflow-x: auto; scrollbar-width: none; }
  .mood-row::-webkit-scrollbar { display: none; }
  .mood-chip { flex: 0 0 auto; padding: 15px 28px; border: 1px solid #ffffff10; border-radius: 14px; background: #ffffff0b; color: #d8d9dc; font-size: 13.5px; font-weight: 800; letter-spacing: .2px; cursor: pointer; transition: background .18s, color .18s, border-color .18s, transform .18s, box-shadow .18s; }
  .mood-chip:hover { border-color: transparent; background: var(--chip-color, #ffffff1c); color: #07150b; transform: translateY(-2px); box-shadow: 0 8px 22px color-mix(in srgb, var(--chip-color, #ffffff) 35%, transparent); }
  .mood-chip.active { border-color: transparent; background: var(--accent); color: #07150b; box-shadow: 0 8px 22px #1ed76040; }
  .skeleton.tile { height: 52px; }

  .editorial-section { margin: 0 0 42px; }
  .section-title { display: flex; align-items: end; justify-content: space-between; margin-bottom: 16px; }
  .section-title h2 { margin: 4px 0 0; font-size: 23px; letter-spacing: -.8px; }
  .section-title button { padding: 6px; border: 0; background: transparent; color: #9b9b9b; font-size: 10px; font-weight: 800; cursor: pointer; }.section-title button:hover { color: #fff; }
  .quick-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 9px; }
  .quick-grid button { display: grid; grid-template-columns: 52px minmax(0, 1fr) 34px; align-items: center; gap: 11px; min-width: 0; padding: 0 10px 0 0; overflow: hidden; border: 1px solid #ffffff08; border-radius: 9px; background: #ffffff0b; backdrop-filter: blur(12px); text-align: left; cursor: pointer; transition: background .18s, transform .18s; }
  .quick-grid button:hover { background: #ffffff18; transform: translateY(-2px); }.quick-grid img, .mini-cover { width: 52px; height: 52px; object-fit: cover; }.mini-cover { display: grid; place-items: center; background: #1ed760; color: #07150b; }.quick-grid strong { overflow: hidden; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }.quick-grid svg { display: grid; width: 28px; height: 28px; padding: 7px; border-radius: 50%; background: var(--accent); color: #07150b; fill: currentColor; stroke: none; opacity: 0; transform: translateY(3px); transition: .18s; }.quick-grid button:hover svg { opacity: 1; transform: none; }
  .editorial-shelf { display: grid; grid-template-columns: repeat(6, minmax(115px, 1fr)); gap: 14px; }
  .editorial-shelf button { min-width: 0; padding: 0; border: 0; background: transparent; text-align: left; cursor: pointer; }.editorial-shelf img, .shelf-cover { width: 100%; aspect-ratio: 1; margin-bottom: 10px; border-radius: 12px; object-fit: cover; box-shadow: 0 15px 35px #0007; transition: transform .22s, box-shadow .22s; }.editorial-shelf button:hover img, .editorial-shelf button:hover .shelf-cover { transform: translateY(-5px); box-shadow: 0 22px 45px #000a; }.shelf-cover { display: grid; place-items: center; background: linear-gradient(145deg, #254b32, #121212); color: var(--accent); font-size: 30px; }.editorial-shelf strong, .editorial-shelf small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.editorial-shelf strong { font-size: 11px; }.editorial-shelf small { margin-top: 4px; color: #7d7d7d; font-size: 9px; }
  .now-playing-expanded { --np-accent: #294532; position: fixed; inset: 8px 8px 96px; z-index: 4; display: grid; grid-template-columns: minmax(320px, .9fr) minmax(380px, 1.1fr); gap: clamp(35px, 7vw, 110px); padding: clamp(48px, 7vh, 90px) clamp(45px, 8vw, 130px); overflow: hidden; border-radius: 10px; background: radial-gradient(circle at 20% 10%, var(--np-accent) 0, #17251b 30%, #121212 62%); box-shadow: 0 25px 80px #000b; transition: background 1s ease; }
  .collapse-now-playing { position: absolute; top: 18px; left: 22px; display: grid; place-items: center; width: 38px; height: 38px; padding: 0; border: 0; border-radius: 50%; background: #0008; color: #fff; cursor: pointer; }
  .collapse-now-playing:hover { background: #000; scale: 1.05; }
  .focus-toggle { position: absolute; top: 18px; right: 22px; display: grid; place-items: center; width: 38px; height: 38px; padding: 0; border: 0; border-radius: 50%; background: #0008; color: #fff; cursor: pointer; z-index: 2; }
  .focus-toggle:hover { background: #000; scale: 1.05; }
  .focus-toggle svg { width: 17px; }
  .app-shell.focus-mode { grid-template-rows: minmax(0, 1fr) 0; padding: 0; }
  .app-shell.focus-mode .player-dock { display: none; }
  .app-shell.focus-mode .now-playing-expanded { inset: 0; border-radius: 0; }
  .expanded-artwork { min-width: 0; display: flex; flex-direction: column; justify-content: center; }
  .expanded-artwork > img { width: min(100%, 560px); aspect-ratio: 1; align-self: center; border-radius: 10px; object-fit: cover; box-shadow: 0 32px 75px #000b; }
  .expanded-meta { width: min(100%, 560px); align-self: center; margin-top: 24px; }
  .expanded-meta span, .lyrics-heading > span { color: var(--accent); font-size: 10px; font-weight: 900; letter-spacing: 1.6px; }
  .expanded-meta h2 { margin: 7px 0 2px; overflow: hidden; font-size: clamp(24px, 3vw, 42px); letter-spacing: -1.3px; text-overflow: ellipsis; white-space: nowrap; }
  .expanded-meta p { margin: 0; color: #b3b3b3; font-size: 15px; }
  .lyrics-panel { min-width: 0; display: flex; flex-direction: column; overflow: hidden; }
  .lyrics-heading { display: flex; align-items: center; justify-content: space-between; flex: 0 0 auto; padding: 4px 8px 20px; }
  .lyrics-heading small { padding: 5px 8px; border-radius: 999px; background: #1ed76020; color: var(--accent); font-size: 8px; font-weight: 900; letter-spacing: 1px; }
  .lyrics-scroll { flex: 1; overflow-y: auto; padding: 35vh 10px; scroll-behavior: smooth; scrollbar-width: none; mask-image: linear-gradient(transparent 0, #000 18%, #000 82%, transparent 100%); }
  .lyrics-scroll::-webkit-scrollbar { display: none; }
  .lyrics-scroll button { display: block; width: 100%; padding: 10px 0; border: 0; background: transparent; color: #ffffff50; font-size: clamp(20px, 2.4vw, 34px); font-weight: 750; line-height: 1.18; letter-spacing: -.7px; text-align: left; cursor: pointer; transition: color .25s, transform .25s; }
  .lyrics-scroll button:hover { color: #ffffffb0; }
  .lyrics-scroll button.active { color: #fff; transform: translateX(8px); }
  .lyrics-scroll button.past { color: #ffffff85; }
  .lyrics-scroll button:disabled { cursor: default; }
  .lyrics-message, .lyrics-loading { height: 100%; display: flex; align-items: center; justify-content: center; color: #a7a7a7; font-size: 15px; text-align: center; }
  .lyrics-message { flex-direction: column; gap: 14px; }
  .lyrics-scroll .lyrics-message button { width: auto; padding: 9px 16px; border-radius: 999px; background: #fff; color: #000; font-size: 11px; font-weight: 800; line-height: 1; }
  .lyrics-scroll .lyrics-message button:hover { color: #000; background: #e7e7e7; }
  .lyrics-loading { gap: 6px; }
  .lyrics-loading span { width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: lyric-pulse .8s ease-in-out infinite alternate; }
  .lyrics-loading span:nth-child(2) { animation-delay: -.3s; }.lyrics-loading span:nth-child(3) { animation-delay: -.6s; }
  @keyframes lyric-pulse { to { opacity: .2; transform: translateY(-7px); } }
  .player-dock { position: relative; grid-column: 1 / 3; grid-row: 2; display: grid; grid-template-columns: minmax(190px, 1fr) minmax(350px, 1.5fr) minmax(190px, 1fr); align-items: center; gap: 20px; padding: 10px 16px; background: #000; z-index: 5; }
  .player-dock > :not(.dock-expand) { position: relative; z-index: 1; }
  .dock-expand { position: absolute; inset: 0; padding: 0; border: 0; background: transparent; cursor: pointer; }
  .dock-left { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .dock-track { display: flex; flex: 1; align-items: center; gap: 11px; min-width: 0; padding: 0; border: 0; background: transparent; text-align: left; cursor: pointer; }
  .dock-like { display: grid; flex: 0 0 34px; place-items: center; width: 34px; height: 34px; padding: 0; border: 0; border-radius: 50%; background: transparent; color: #71747d; cursor: pointer; transition: color .15s, transform .15s; }
  .dock-like:hover { color: #fff; transform: scale(1.1); }
  .dock-like.liked { color: var(--accent); }
  .dock-like.liked svg { fill: currentColor; }
  .dock-like svg { width: 17px; height: 17px; }
  .dock-track:hover strong { text-decoration: underline; }
  .dock-track:disabled { opacity: 1; cursor: default; }
  .dock-track img, .dock-cover { width: 54px; height: 54px; flex: 0 0 54px; border-radius: 8px; object-fit: cover; }
  .dock-cover.app-mark { border-radius: 50%; background: linear-gradient(145deg, #35ed7a, #1db954); color: #fff; box-shadow: 0 8px 22px #1ed76030, inset 0 1px #ffffff45; }
  .app-mark svg { width: 26px; height: 26px; fill: currentColor; stroke: none; }
  .dock-cover.idle { color: #fff; }
  .dock-track > div:last-child { min-width: 0; display: grid; gap: 4px; }
  .dock-track strong, .dock-track span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dock-track strong { font-size: 12px; }.dock-track span { color: #747780; font-size: 10px; }
  .transport { display: grid; gap: 7px; }
  .transport-buttons { display: flex; justify-content: center; align-items: center; gap: 14px; }
  .transport button { display: grid; place-items: center; width: 28px; height: 28px; padding: 0; border: 0; background: transparent; color: #b6b8bf; cursor: pointer; }
  .transport button svg { width: 17px; fill: currentColor; stroke: none; }
  .transport .main-play { width: 36px; height: 36px; border-radius: 50%; background: #f4f4f5; color: #0b0c0e; box-shadow: 0 5px 18px #0008; }
  .transport .main-play:hover { scale: 1.05; }
  .transport .seek-ten { position: relative; }
  .transport .seek-ten svg { width: 22px; fill: none; stroke: currentColor; }
  .transport .seek-ten span { position: absolute; top: 9px; font-size: 7px; font-weight: 800; }
  .spinner { width: 15px; height: 15px; border: 2px solid #aaa; border-top-color: #111; border-radius: 50%; animation: spin .7s linear infinite; }
  @keyframes spin { to { rotate: 360deg; } }
  .timeline { display: grid; grid-template-columns: 34px 1fr 34px; align-items: center; gap: 8px; }
  .timeline time { color: #a7a7a7; font-size: 9px; text-align: center; }
  .scrubber { width: 100%; height: 4px; appearance: none; border-radius: 3px; background: linear-gradient(to right, var(--accent) var(--progress), #4d4d4d var(--progress)); cursor: pointer; }
  .scrubber::-webkit-slider-thumb { width: 12px; height: 12px; appearance: none; border-radius: 50%; background: #fff; opacity: 0; transition: opacity .15s; }
  .scrubber:hover::-webkit-slider-thumb, .scrubber:focus::-webkit-slider-thumb { opacity: 1; }
  .scrubber:disabled { cursor: default; opacity: .45; }
  .volume { display: flex; justify-content: end; align-items: center; gap: 9px; color: #71747d; }
  .volume svg { width: 18px; }
  .volume-toggle { display: grid; place-items: center; padding: 4px; border: 0; border-radius: 6px; background: transparent; color: inherit; cursor: pointer; transition: color .15s, background .15s; }
  .volume-toggle:hover { color: #fff; background: #ffffff14; }
  .volume input[type="range"] { width: 92px; height: 4px; appearance: none; border-radius: 3px; background: linear-gradient(to right, var(--accent) var(--volume), #4d4d4d var(--volume)); }
  .volume input[type="range"]::-webkit-slider-thumb { width: 10px; height: 10px; appearance: none; border-radius: 50%; background: #fff; cursor: pointer; }
  .volume span { width: 22px; font-size: 9px; }
  .dock-radio { display: grid; place-items: center; padding: 4px; border: 0; border-radius: 6px; background: transparent; color: inherit; cursor: pointer; transition: color .15s, background .15s; }
  .dock-radio:hover { color: #fff; background: #ffffff14; }
  .dock-radio.active { color: var(--accent); }
  .settings-popover { position: absolute; right: 0; top: calc(100% + 12px); z-index: 60; display: grid; gap: 14px; min-width: 280px; max-width: 320px; max-height: 60vh; overflow-y: auto; padding: 16px; border: 1px solid #ffffff17; border-radius: 14px; background: #1b1b1df2; box-shadow: 0 18px 50px #000a; backdrop-filter: blur(18px); }
  .settings-row { display: grid; gap: 8px; color: #c8cad0; font-size: 11px; font-weight: 650; }
  .settings-row > span { color: #8d9099; font-size: 9px; font-weight: 800; letter-spacing: 1px; text-transform: uppercase; white-space: nowrap; }
  .settings-toggle { display: flex; align-items: center; gap: 9px; cursor: pointer; color: #c8cad0; font-size: 11px; font-weight: 650; white-space: nowrap; }
  .settings-toggle input { width: 15px; height: 15px; margin: 0; accent-color: var(--accent); cursor: pointer; }
  .settings-toggle input:checked { accent-color: var(--accent); }
  .sleep-options { display: flex; gap: 6px; }
  .sleep-options button { padding: 5px 10px; border: 1px solid #ffffff1f; border-radius: 999px; background: transparent; color: #c8cad0; font-size: 10px; font-weight: 700; cursor: pointer; }
  .sleep-options button:hover { border-color: var(--accent); color: #fff; }
  .sleep-countdown { color: var(--accent); font-size: 10px; }
  .settings-popover select { padding: 7px 9px; border: 1px solid #ffffff1f; border-radius: 8px; background: #141416; color: #e8e9ec; font-size: 11px; }
  .remote-popover { display: grid; justify-items: center; gap: 8px; min-width: 0; padding: 14px 18px; }
  .remote-popover .settings-row { justify-items: center; text-align: center; }
  .remote-qr { display: grid; place-items: center; padding: 6px; border-radius: 10px; background: #1b1d24; }
  .remote-qr :global(svg) { width: 132px; height: 132px; display: block; }
  .remote-hint { max-width: 200px; color: #8d9099; font-size: 10px; text-align: center; line-height: 1.5; }
  .remote-url { color: #4d5058; font-size: 9px; word-break: break-all; }
  .back-button { display: inline-flex; align-items: center; gap: 6px; margin-bottom: 14px; padding: 7px 14px; border: 1px solid #ffffff1a; border-radius: 999px; background: transparent; color: #b7bac2; font-size: 11px; font-weight: 700; cursor: pointer; }
  .back-button:hover { border-color: #fff; color: #fff; }
  .back-button svg { width: 14px; }
  .entity-hero { display: flex; align-items: center; gap: 20px; margin-bottom: 22px; }
  .entity-hero img, .entity-fallback { width: 160px; height: 160px; flex: 0 0 160px; border-radius: 14px; object-fit: cover; box-shadow: 0 18px 46px #0009; }
  .entity-fallback { display: grid; place-items: center; background: linear-gradient(145deg, #245832, #171817); color: var(--accent); font-size: 42px; }
  .entity-hero h2 { margin: 6px 0 0; font-size: 30px; letter-spacing: -.5px; }
  .entity-hero p { margin: 6px 0 0; color: #9a9da6; font-size: 12px; }
  .shelf-title { margin: 4px 2px 12px; color: #b7bac2; font-size: 13px; font-weight: 750; }
  .meta-link { padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; }
  .meta-link:hover { color: #fff; text-decoration: underline; }
  :global(::selection) { background: #1ed76055; color: #fff; }
  :global(button:focus-visible), :global(input:focus-visible) { outline: 2px solid #fff; outline-offset: 3px; }
  .sidebar, .content { border: 1px solid #ffffff08; box-shadow: inset 0 1px #ffffff08, 0 12px 40px #0005; }
  .content { position: relative; isolation: isolate; background: linear-gradient(180deg, #252825 0, #191a19 145px, #121212 340px); }
  .content::before { content: ""; position: absolute; z-index: -1; inset: 0; pointer-events: none; background: radial-gradient(circle at 78% -12%, #1ed76013, transparent 35%); }
  .topbar { position: relative; z-index: 2; box-shadow: 0 8px 30px #0002; }
  .search-input { box-shadow: inset 0 1px #ffffff0d, 0 5px 18px #0003; }
  .search-input::placeholder { color: #8c8c8c; }
  .play-all, .main-play { transition: transform .16s ease, filter .16s ease, box-shadow .16s ease; }
  .play-all:active, .main-play:active { transform: scale(.96); }
  .play-all { box-shadow: 0 8px 24px #1ed76020; }
  .stage-heading { padding: 8px 2px 4px; }
  .stage-heading h1 { text-wrap: balance; text-shadow: 0 2px 20px #0008; }
  .track-row { position: relative; transition: background .16s ease, transform .16s ease, box-shadow .16s ease; }
  .track-row:hover { transform: translateX(3px); box-shadow: 0 7px 22px #0003; }
  .track-row.playing::before { content: ""; position: absolute; left: 0; width: 3px; height: 24px; border-radius: 3px; background: var(--accent); box-shadow: 0 0 12px #1ed76080; }
  .track-title-cell img, .cover-fallback, .dock-track img, .dock-cover { box-shadow: 0 5px 16px #0006; }
  .player-dock { border-top: 1px solid #ffffff0d; box-shadow: 0 -12px 36px #0008; }
  .transport button:not(.main-play) { transition: color .15s ease, transform .15s ease; }
  .transport button:not(.main-play):hover:not(:disabled) { color: #fff; transform: scale(1.08); }
  .transport .main-play { box-shadow: 0 6px 22px #000a, inset 0 1px #fff; }
  .scrubber, .volume input[type="range"] { transition: height .15s ease, filter .15s ease; }
  .scrubber:hover, .volume input[type="range"]:hover { filter: brightness(1.12); }
  .now-playing-expanded { animation: expand-player .28s cubic-bezier(.2,.8,.2,1); }
  @keyframes expand-player { from { opacity: 0; transform: scale(.985) translateY(8px); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) { *, *::before, *::after { scroll-behavior: auto !important; animation-duration: .01ms !important; transition-duration: .01ms !important; } }
  @media (max-width: 1200px) { .editorial-shelf { grid-template-columns: repeat(4, minmax(115px, 1fr)); }.quick-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  @media (max-width: 1050px) { .app-shell { grid-template-columns: 180px minmax(430px, 1fr); }.app-shell.sidebar-collapsed { grid-template-columns: 72px minmax(430px, 1fr); }.player-dock { grid-column: 1 / 3; }.now-playing-expanded { grid-template-columns: minmax(260px, .8fr) minmax(330px, 1.2fr); gap: 34px; padding-inline: 55px; }.editorial-hero { grid-template-columns: minmax(0, 1fr) 240px; }.editorial-shelf { grid-template-columns: repeat(4, minmax(105px, 1fr)); } }
</style>
