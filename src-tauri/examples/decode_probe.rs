use std::io::{Read, Seek, SeekFrom};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_AAC, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::default::{get_codecs, get_probe};

struct FileSource(std::fs::File, u64);
impl MediaSource for FileSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.1)
    }
}
impl Read for FileSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl Seek for FileSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

fn main() {
    for arg in std::env::args().skip(1) {
        let f = std::fs::File::open(&arg).unwrap();
        let len = f.metadata().unwrap().len();
        let mss = MediaSourceStream::new(Box::new(FileSource(f, len)), Default::default());
        let r = get_probe().format(
            &Default::default(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        );
        match r {
            Ok(mut p) => {
                let tracks: Vec<String> = p
                    .format
                    .tracks()
                    .iter()
                    .map(|t| format!("id={} codec={:?}", t.id, t.codec_params.codec))
                    .collect();
                println!("{arg}: tracks {tracks:?}");
                let track = p
                    .format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec == CODEC_TYPE_AAC)
                    .or_else(|| {
                        p.format
                            .tracks()
                            .iter()
                            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
                    })
                    .unwrap()
                    .clone();
                let tid = track.id;
                let mut dec = get_codecs()
                    .make(&track.codec_params, &DecoderOptions::default())
                    .unwrap();
                let mut n = 0usize;
                let mut samples = 0usize;
                loop {
                    match p.format.next_packet() {
                        Ok(packet) => {
                            if packet.track_id() != tid {
                                continue;
                            }
                            match dec.decode(&packet) {
                                Ok(buf) => {
                                    n += 1;
                                    samples += buf.frames();
                                    if n > 20 {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    println!("  decode err {e}");
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            println!("  packets end: {e}");
                            break;
                        }
                    }
                }
                println!("  decoded {n} packets, {samples} frames");
            }
            Err(e) => println!("{arg}: PROBE ERR {e}"),
        }
    }
}
