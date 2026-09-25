use std::collections::HashMap;
use std::f64::consts::TAU;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
use windows::core::PCWSTR;

use crate::status::Cue;

const SAMPLE_RATE: u32 = 22_050;
const FADE_MS: f64 = 5.0;
const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tone {
    f0: f64,
    f1: f64,
    ms: f64,
    gap_ms: f64,
}

const fn tone(f: f64, ms: f64, gap_ms: f64) -> Tone {
    Tone {
        f0: f,
        f1: f,
        ms,
        gap_ms,
    }
}

fn tones(cue: Cue) -> &'static [Tone] {
    const CAPTURE_ON: [Tone; 2] = [tone(660.0, 70.0, 30.0), tone(880.0, 90.0, 0.0)];
    const CAPTURE_OFF: [Tone; 2] = [tone(880.0, 70.0, 30.0), tone(660.0, 90.0, 0.0)];
    const WARNING: [Tone; 1] = [tone(440.0, 160.0, 0.0)];
    const LOST: [Tone; 3] = [
        tone(330.0, 70.0, 50.0),
        tone(330.0, 70.0, 50.0),
        tone(330.0, 70.0, 0.0),
    ];
    const RECONNECTED: [Tone; 3] = [
        tone(523.0, 60.0, 20.0),
        tone(659.0, 60.0, 20.0),
        tone(784.0, 90.0, 0.0),
    ];
    const PROFILE: [Tone; 1] = [Tone {
        f0: 1200.0,
        f1: 1800.0,
        ms: 70.0,
        gap_ms: 0.0,
    }];
    const BIND: [Tone; 1] = [tone(1000.0, 100.0, 0.0)];
    match cue {
        Cue::CaptureOn => &CAPTURE_ON,
        Cue::CaptureOff => &CAPTURE_OFF,
        Cue::Warning => &WARNING,
        Cue::Lost => &LOST,
        Cue::Reconnected => &RECONNECTED,
        Cue::ProfileChanged => &PROFILE,
        Cue::Bind => &BIND,
    }
}

fn samples_for(ms: f64) -> usize {
    (ms / 1000.0 * f64::from(SAMPLE_RATE)).round() as usize
}

fn render_tone(t: &Tone, amplitude: f64, out: &mut Vec<i16>) {
    let n = samples_for(t.ms);
    let fade = samples_for(FADE_MS).max(1);
    let mut phase = 0.0_f64;
    for i in 0..n {
        let progress = i as f64 / n.max(1) as f64;
        let freq = t.f0 + (t.f1 - t.f0) * progress;
        phase = (phase + TAU * freq / f64::from(SAMPLE_RATE)) % TAU;
        let edge = i.min(n - 1 - i);
        let envelope = (edge as f64 / fade as f64).min(1.0);
        out.push((phase.sin() * amplitude * envelope) as i16);
    }
    out.extend(std::iter::repeat_n(0, samples_for(t.gap_ms)));
}

pub fn synthesize(cue: Cue, volume_pct: u8) -> Vec<u8> {
    let amplitude = f64::from(volume_pct.min(100)) / 100.0 * 0.5 * f64::from(i16::MAX);
    let mut samples = Vec::new();
    for t in tones(cue) {
        render_tone(t, amplitude, &mut samples);
    }
    wav_bytes(&samples)
}

fn wav_bytes(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub struct SoundPlayer {
    tx: SyncSender<(Cue, u8)>,
}

impl SoundPlayer {
    pub fn start() -> std::io::Result<Self> {
        let (tx, rx) = sync_channel(QUEUE);
        std::thread::Builder::new()
            .name("sound".into())
            .spawn(move || worker(rx))?;
        Ok(Self { tx })
    }

    pub fn play(&self, cue: Cue, volume_pct: u8) {
        if volume_pct == 0 {
            return;
        }
        match self.tx.try_send((cue, volume_pct)) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => crate::log::line("ses thread'i kapalı"),
        }
    }
}

fn worker(rx: Receiver<(Cue, u8)>) {
    let mut cache: HashMap<(Cue, u8), Vec<u8>> = HashMap::new();
    for key in rx {
        let wav = cache.entry(key).or_insert_with(|| synthesize(key.0, key.1));
        // SAFETY: SND_MEMORY ile işaretçi bellek içi WAV görüntüsünü gösterir;
        // SND_SYNC olduğu için arabellek çalma bitene kadar yaşar.
        unsafe {
            let _ = PlaySoundW(
                PCWSTR(wav.as_ptr().cast()),
                None,
                SND_MEMORY | SND_SYNC | SND_NODEFAULT,
            );
        }
    }
}
