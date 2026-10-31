use std::io::{Read as IoRead, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use rodio::{DeviceTrait, OutputStream, Sink, Source};
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_AAC, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, SeekedTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;
use symphonia::default::{get_codecs, get_probe};

const MAX_DECODE_RETRIES: usize = 3;

/// MediaSource over a file that reports its byte length; rodio's own wrapper
/// reports `byte_len = None` which makes Symphonia's MP4 probe fail.
struct FileSource(std::fs::File);

impl MediaSource for FileSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.0.metadata().ok().map(|m| m.len())
    }
}

impl IoRead for FileSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for FileSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

/// Decoded-audio `Source` backed by Symphonia.
struct AudioSource {
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    format: Box<dyn FormatReader>,
    track_id: u32,
    total_duration: Option<Duration>,
    spec: SignalSpec,
    buffer: SampleBuffer<i16>,
    offset: usize,
}

impl AudioSource {
    fn open(path: &std::path::Path) -> Result<Self, String> {
        let file =
            std::fs::File::open(path).map_err(|e| format!("failed to open audio file: {e}"))?;
        let mss = MediaSourceStream::new(Box::new(FileSource(file)), Default::default());
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let mut probed = get_probe()
            .format(
                &Default::default(),
                mss,
                &format_opts,
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("audio probe failed: {e}"))?;
        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec == CODEC_TYPE_AAC)
            .or_else(|| {
                probed
                    .format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            })
            .ok_or_else(|| "no supported audio track".to_string())?;
        let track_id = track.id;
        let mut decoder = get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("audio codec unsupported: {e}"))?;
        let total_duration = track
            .codec_params
            .time_base
            .zip(track.codec_params.n_frames)
            .map(|(base, frames)| {
                let t = base.calc_time(frames);
                Duration::new(t.seconds, (1f64 / t.frac) as u32)
            });
        let mut errors = 0;
        let decoded = loop {
            match probed.format.next_packet() {
                Ok(packet) if packet.track_id() == track_id => match decoder.decode(&packet) {
                    Ok(d) => break d,
                    Err(SymphoniaError::DecodeError(_)) if errors < MAX_DECODE_RETRIES => {
                        errors += 1;
                    }
                    Err(e) => return Err(format!("audio decode failed: {e}")),
                },
                Ok(_) => {}
                Err(SymphoniaError::IoError(_)) => break decoder.last_decoded(),
                Err(e) => return Err(format!("audio packet error: {e}")),
            }
        };
        let spec = *decoded.spec();
        let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        Ok(AudioSource {
            decoder,
            format: probed.format,
            track_id,
            total_duration,
            spec,
            buffer,
            offset: 0,
        })
    }

    fn refine_position(&mut self, seek_res: SeekedTo) -> Result<(), String> {
        let mut to_pass = seek_res.required_ts.saturating_sub(seek_res.actual_ts);
        let packet = loop {
            let candidate = self
                .format
                .next_packet()
                .map_err(|e| format!("audio packet error after seek: {e}"))?;
            if candidate.dur() > to_pass {
                break candidate;
            }
            to_pass -= candidate.dur();
        };
        let mut decoded = self.decoder.decode(&packet);
        for _ in 0..MAX_DECODE_RETRIES {
            if decoded.is_err() {
                let packet = self
                    .format
                    .next_packet()
                    .map_err(|e| format!("audio packet error after seek: {e}"))?;
                decoded = self.decoder.decode(&packet);
            }
        }
        let decoded = decoded.map_err(|e| format!("audio decode failed after seek: {e}"))?;
        self.spec = *decoded.spec();
        self.buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, self.spec);
        self.buffer.copy_interleaved_ref(decoded);
        self.offset = to_pass as usize * self.spec.channels.count();
        Ok(())
    }
}

impl Iterator for AudioSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.offset >= self.buffer.len() {
            let packet = self.format.next_packet().ok()?;
            let mut decoded = self.decoder.decode(&packet);
            for _ in 0..MAX_DECODE_RETRIES {
                if decoded.is_err() {
                    let packet = self.format.next_packet().ok()?;
                    decoded = self.decoder.decode(&packet);
                }
            }
            let decoded = decoded.ok()?;
            self.spec = *decoded.spec();
            self.buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, self.spec);
            self.buffer.copy_interleaved_ref(decoded);
            self.offset = 0;
        }
        let sample = *self.buffer.samples().get(self.offset)?;
        self.offset += 1;
        Some(sample)
    }
}

impl Source for AudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.buffer.samples().len().saturating_sub(self.offset))
    }
    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }
    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        let time: Time = pos.as_secs_f64().into();
        let res = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| {
                rodio::source::SeekError::SymphoniaDecoder(
                    rodio::decoder::symphonia::SeekError::BaseSeek(e),
                )
            })?;
        self.refine_position(res).map_err(|_| {
            rodio::source::SeekError::SymphoniaDecoder(
                rodio::decoder::symphonia::SeekError::Refining(SymphoniaError::DecodeError(
                    "seek refinement failed",
                )),
            )
        })
    }
}

fn open_audio(path: &std::path::Path) -> Result<AudioSource, String> {
    AudioSource::open(path)
}

/// Panic-safe decode: the audio thread must never die on a bad file.
fn decode_audio(path: &std::path::Path) -> Result<AudioSource, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open_audio(path)))
        .map_err(|_| "audio decoder panicked".to_string())
        .and_then(|r| r)
}

#[derive(Clone, Default, serde::Serialize)]
pub struct PlaybackStats {
    pub status: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// Path of the file currently feeding the output; changes on rollover so
    /// the frontend can detect gapless/crossfade track transitions.
    pub path: String,
}

enum Cmd {
    Play {
        path: PathBuf,
        duration_ms: u64,
        result: Sender<Result<(), String>>,
    },
    /// Queues the next track so it starts without a gap. With `crossfade_ms`
    /// it plays immediately on a second sink and overlaps the current track's
    /// tail; otherwise it is appended to the current sink for true gapless.
    QueueNext {
        path: PathBuf,
        duration_ms: u64,
        crossfade_ms: u64,
        result: Sender<Result<(), String>>,
    },
    Pause,
    Resume,
    Seek {
        position_ms: u64,
        result: Sender<Result<(), String>>,
    },
    SetVolume(f32),
    SetDevice {
        name: Option<String>,
        result: Sender<Result<(), String>>,
    },
}

/// Names of all available output devices (default device first).
pub fn output_devices() -> Vec<String> {
    use rodio::cpal::traits::HostTrait;
    let host = rodio::cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.name().ok());
    let mut names = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    if let Some(default) = default {
        if let Some(index) = names.iter().position(|n| *n == default) {
            names.swap(0, index);
        }
    }
    names
}

fn open_stream(
    device_name: Option<&str>,
) -> Option<(OutputStream, rodio::OutputStreamHandle, String)> {
    use rodio::cpal::traits::HostTrait;
    let host = rodio::cpal::default_host();
    let device = match device_name {
        Some(name) => host
            .output_devices()
            .ok()?
            .find(|d| d.name().ok().as_deref() == Some(name)),
        None => host.default_output_device(),
    }?;
    let name = device.name().unwrap_or_else(|_| "default".to_string());
    OutputStream::try_from_device(&device)
        .ok()
        .map(|(stream, handle)| (stream, handle, name))
}

/// Channel-based handle to the audio thread. rodio's OutputStream/Sink are
/// !Send, so they live on a dedicated thread; this handle is Send + Sync.
pub struct Player {
    tx: Sender<Cmd>,
    stats: Arc<RwLock<PlaybackStats>>,
}

impl Player {
    pub fn new() -> Self {
        let (tx, rx) = channel::<Cmd>();
        let stats = Arc::new(RwLock::new(PlaybackStats {
            status: "stopped".into(),
            ..Default::default()
        }));
        let thread_stats = stats.clone();

        thread::spawn(move || {
            // The output device can be briefly unavailable at launch (driver
            // init, device reconnect). Retry rather than killing the thread,
            // which would leave every command timing out forever.
            let (mut _stream, mut handle, mut sink) = {
                let mut attempt = 0;
                loop {
                    if let Some((stream, handle, _)) = open_stream(None) {
                        if let Ok(sink) = Sink::try_new(&handle) {
                            break (stream, handle, sink);
                        }
                    }
                    attempt += 1;
                    if attempt > 60 {
                        panic!("no audio output device became available");
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            };
            let mut volume: f32 = 0.8;
            sink.set_volume(volume);
            let mut duration_ms: u64 = 0;
            let mut current_path: Option<PathBuf> = None;
            // Crossfade: a second sink overlapping the current track's tail.
            let mut next_sink: Option<(Sink, PathBuf, u64)> = None;
            // Gapless: a source appended to the live sink; metadata applies
            // once the sink advances to it.
            let mut pending_rollover: Option<(u64, PathBuf)> = None;
            let mut queued_next = false;

            loop {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Cmd::Play {
                            path,
                            duration_ms: d,
                            result,
                        } => {
                            let decoded = decode_audio(&path);

                            match decoded {
                                Ok(src) => {
                                    // A dead device kills try_new; reopen the
                                    // stream once before giving up so the
                                    // audio thread itself never panics.
                                    let fresh = match Sink::try_new(&handle) {
                                        Ok(s) => s,
                                        Err(e) => match open_stream(None) {
                                            Some((stream, new_handle, _)) => {
                                                _stream = stream;
                                                handle = new_handle;
                                                match Sink::try_new(&handle) {
                                                    Ok(s) => s,
                                                    Err(e2) => {
                                                        let _ = result.send(Err(format!(
                                                            "audio sink unavailable: {e2}"
                                                        )));
                                                        continue;
                                                    }
                                                }
                                            }
                                            None => {
                                                let _ = result
                                                    .send(Err(format!("audio output lost: {e}")));
                                                continue;
                                            }
                                        },
                                    };
                                    fresh.set_volume(volume);
                                    fresh.append(src);
                                    fresh.play();
                                    sink.stop();
                                    sink = fresh;
                                    if let Some((old, _, _)) = next_sink.take() {
                                        old.stop();
                                    }
                                    pending_rollover = None;
                                    queued_next = false;
                                    duration_ms = d;
                                    current_path = Some(path);
                                    let _ = result.send(Ok(()));
                                }
                                Err(error) => {
                                    let _ = result.send(Err(error));
                                }
                            }
                        }
                        Cmd::QueueNext {
                            path,
                            duration_ms: d,
                            crossfade_ms,
                            result,
                        } => {
                            if queued_next || sink.empty() {
                                let _ = result.send(Ok(()));
                                continue;
                            }
                            let decoded = decode_audio(&path);
                            match decoded {
                                Ok(src) => {
                                    if crossfade_ms > 0 {
                                        let fresh = match Sink::try_new(&handle) {
                                            Ok(s) => s,
                                            Err(_) => {
                                                let _ = result.send(Err(
                                                    "audio sink unavailable for crossfade"
                                                        .to_string(),
                                                ));
                                                continue;
                                            }
                                        };
                                        fresh.set_volume(volume);
                                        fresh.append(src);
                                        fresh.play();
                                        next_sink = Some((fresh, path, d));
                                    } else {
                                        sink.append(src);
                                        pending_rollover = Some((d, path));
                                    }
                                    queued_next = true;
                                    let _ = result.send(Ok(()));
                                }
                                Err(error) => {
                                    let _ = result.send(Err(error));
                                }
                            }
                        }
                        Cmd::Pause => {
                            sink.pause();
                            if let Some((next, _, _)) = &next_sink {
                                next.pause();
                            }
                        }
                        Cmd::Resume => {
                            sink.play();
                            if let Some((next, _, _)) = &next_sink {
                                next.play();
                            }
                        }
                        Cmd::Seek {
                            position_ms,
                            result,
                        } => {
                            let target = Duration::from_millis(position_ms);
                            let seek_result: Result<(), String> = sink
                                .try_seek(target)
                                .map_err(|error| error.to_string())
                                .or_else(|seek_error| {
                                    let path = current_path
                                        .as_ref()
                                        .ok_or_else(|| "no track is loaded".to_string())?;
                                    let source = decode_audio(path).map_err(|e| {
                                        format!("failed to decode audio for seek: {e}")
                                    })?;
                                    let was_paused = sink.is_paused();
                                    let fresh = Sink::try_new(&handle)
                                        .map_err(|e| format!("failed to create audio sink: {e}"))?;
                                    fresh.set_volume(volume);
                                    fresh.append(source.skip_duration(target));
                                    if was_paused {
                                        fresh.pause();
                                    } else {
                                        fresh.play();
                                    }
                                    sink.stop();
                                    sink = fresh;
                                    let _ = seek_error;
                                    Ok(())
                                });
                            let _ = result.send(seek_result);
                        }
                        Cmd::SetVolume(v) => {
                            volume = v.clamp(0.0, 2.0);
                            sink.set_volume(volume);
                            if let Some((next, _, _)) = &next_sink {
                                next.set_volume(volume);
                            }
                        }
                        Cmd::SetDevice { name, result } => {
                            let outcome = (|| -> Result<(), String> {
                                let (stream, new_handle, _new_name) = open_stream(name.as_deref())
                                    .ok_or_else(|| "audio device unavailable".to_string())?;
                                let was_paused = sink.is_paused() || sink.empty();
                                let position = sink.get_pos();
                                sink.stop();
                                if let Some((next, _, _)) = next_sink.take() {
                                    next.stop();
                                }
                                pending_rollover = None;
                                queued_next = false;
                                let fresh = Sink::try_new(&new_handle)
                                    .map_err(|e| format!("failed to create audio sink: {e}"))?;
                                fresh.set_volume(volume);
                                if let Some(path) = current_path.as_ref() {
                                    if let Ok(source) = decode_audio(path) {
                                        fresh.append(source.skip_duration(position));
                                        if was_paused {
                                            fresh.pause();
                                        } else {
                                            fresh.play();
                                        }
                                    }
                                }
                                _stream = stream;
                                handle = new_handle;
                                sink = fresh;
                                Ok(())
                            })();
                            let _ = result.send(outcome);
                        }
                    }
                }

                // Gapless rollover: appended source just became current.
                if pending_rollover.is_some() && sink.len() <= 1 {
                    if let Some((d, p)) = pending_rollover.take() {
                        duration_ms = d;
                        current_path = Some(p);
                        queued_next = false;
                    }
                }
                // Crossfade rollover: current sink drained; promote the next.
                if sink.empty() {
                    if let Some((next, p, d)) = next_sink.take() {
                        sink = next;
                        current_path = Some(p);
                        duration_ms = d;
                        queued_next = false;
                    }
                }

                {
                    let mut s = thread_stats.write().unwrap();
                    s.status = if sink.empty() {
                        "stopped".into()
                    } else if sink.is_paused() {
                        "paused".into()
                    } else {
                        "playing".into()
                    };
                    s.position_ms = sink.get_pos().as_millis() as u64;
                    s.duration_ms = duration_ms;
                    s.path = current_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }

                thread::sleep(Duration::from_millis(100));
            }
        });

        Self { tx, stats }
    }

    pub fn play_file(&self, path: PathBuf, duration_ms: u64) -> Result<(), String> {
        let (result_tx, result_rx) = channel();
        self.tx
            .send(Cmd::Play {
                path,
                duration_ms,
                result: result_tx,
            })
            .map_err(|e| format!("audio thread unreachable: {e}"))?;
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("audio thread did not respond: {e}"))?
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(Cmd::Resume);
    }

    pub fn seek(&self, position_ms: u64) -> Result<(), String> {
        let (result_tx, result_rx) = channel();
        self.tx
            .send(Cmd::Seek {
                position_ms,
                result: result_tx,
            })
            .map_err(|e| format!("audio thread unreachable: {e}"))?;
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("audio thread did not respond: {e}"))?
    }

    pub fn queue_next(
        &self,
        path: PathBuf,
        duration_ms: u64,
        crossfade_ms: u64,
    ) -> Result<(), String> {
        let (result_tx, result_rx) = channel();
        self.tx
            .send(Cmd::QueueNext {
                path,
                duration_ms,
                crossfade_ms,
                result: result_tx,
            })
            .map_err(|e| format!("audio thread unreachable: {e}"))?;
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("audio thread did not respond: {e}"))?
    }

    pub fn set_device(&self, name: Option<String>) -> Result<(), String> {
        let (result_tx, result_rx) = channel();
        self.tx
            .send(Cmd::SetDevice {
                name,
                result: result_tx,
            })
            .map_err(|e| format!("audio thread unreachable: {e}"))?;
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("audio thread did not respond: {e}"))?
    }

    pub fn set_volume(&self, volume: f32) {
        let _ = self.tx.send(Cmd::SetVolume(volume));
    }

    pub fn stats(&self) -> PlaybackStats {
        self.stats.read().unwrap().clone()
    }
}
