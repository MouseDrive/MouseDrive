use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

use libloading::Library;

use super::{SAMPLE_RATE, WAV_HEADER_LEN};

const LIBRARY: &str = "libasound.so.2";
const SND_PCM_STREAM_PLAYBACK: c_int = 0;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
const LATENCY_US: c_uint = 50_000;

type Pcm = *mut c_void;
type FnOpen = unsafe extern "C" fn(*mut Pcm, *const c_char, c_int, c_int) -> c_int;
type FnSetParams = unsafe extern "C" fn(Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type FnWritei = unsafe extern "C" fn(Pcm, *const c_void, c_ulong) -> c_long;
type FnRecover = unsafe extern "C" fn(Pcm, c_int, c_int) -> c_int;
type FnPcm = unsafe extern "C" fn(Pcm) -> c_int;

pub(super) struct Output {
    open: FnOpen,
    set_params: FnSetParams,
    writei: FnWritei,
    recover: FnRecover,
    drain: FnPcm,
    close: FnPcm,
    _lib: Library,
}

fn symbol<T: Copy>(lib: &Library, name: &str) -> Option<T> {
    let cname = format!("{name}\0");
    // SAFETY: T, ALSA başlıklarındaki imzayla eşleşen bir işlev işaretçisi
    // tipidir; Library Output içinde tutulduğu için işaretçi geçerli kalır.
    unsafe { lib.get::<T>(cname.as_bytes()) }.ok().map(|s| *s)
}

impl Output {
    pub(super) fn open() -> Option<Self> {
        // SAFETY: libasound yüklenirken yalnız kendi başlatıcıları çalışır.
        let lib = match unsafe { Library::new(LIBRARY) } {
            Ok(lib) => lib,
            Err(e) => {
                crate::log::line(&format!("ALSA yüklenemedi, sesler kapalı: {e}"));
                return None;
            }
        };
        let output = Self {
            open: symbol(&lib, "snd_pcm_open")?,
            set_params: symbol(&lib, "snd_pcm_set_params")?,
            writei: symbol(&lib, "snd_pcm_writei")?,
            recover: symbol(&lib, "snd_pcm_recover")?,
            drain: symbol(&lib, "snd_pcm_drain")?,
            close: symbol(&lib, "snd_pcm_close")?,
            _lib: lib,
        };
        Some(output)
    }

    pub(super) fn play(&self, wav: &[u8]) {
        let Some(pcm_bytes) = wav.get(WAV_HEADER_LEN..) else {
            return;
        };
        let samples: Vec<i16> = pcm_bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        let mut pcm: Pcm = std::ptr::null_mut();
        // SAFETY: pcm yerel bir işaretçi; ad sıfır sonlu sabit bir C dizgisi.
        let opened =
            unsafe { (self.open)(&mut pcm, c"default".as_ptr(), SND_PCM_STREAM_PLAYBACK, 0) };
        if opened < 0 || pcm.is_null() {
            return;
        }
        // SAFETY: pcm az önce açıldı; parametreler ALSA sabitleri.
        let ready = unsafe {
            (self.set_params)(
                pcm,
                SND_PCM_FORMAT_S16_LE,
                SND_PCM_ACCESS_RW_INTERLEAVED,
                1,
                SAMPLE_RATE,
                1,
                LATENCY_US,
            )
        } >= 0;
        if ready {
            self.write_all(pcm, &samples);
            // SAFETY: pcm açık; drain yazılanlar çalınana kadar bekler.
            unsafe { (self.drain)(pcm) };
        }
        // SAFETY: pcm açık ve bundan sonra kullanılmaz.
        unsafe { (self.close)(pcm) };
    }

    fn write_all(&self, pcm: Pcm, samples: &[i16]) {
        let mut rest = samples;
        while !rest.is_empty() {
            // SAFETY: pcm açık; rest geçerli bir dilim ve kare sayısı (mono)
            // dilim uzunluğuna eşit.
            let written =
                unsafe { (self.writei)(pcm, rest.as_ptr().cast(), rest.len() as c_ulong) };
            if written >= 0 {
                rest = &rest[(written as usize).min(rest.len())..];
                continue;
            }
            // SAFETY: pcm açık; recover yetersiz akış gibi durumlardan döner.
            let recovered = unsafe { (self.recover)(pcm, written as c_int, 1) };
            if recovered < 0 {
                return;
            }
        }
    }
}
