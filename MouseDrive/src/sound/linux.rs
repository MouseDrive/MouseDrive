use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void};

use libloading::Library;

use super::{SAMPLE_RATE, WAV_HEADER_LEN};

const LIBRARY: &str = "libasound.so.2";
const DEVICE: &CStr = c"default";
const SND_PCM_STREAM_PLAYBACK: c_int = 0;
const SND_PCM_NONBLOCK: c_int = 1;
const SND_PCM_BLOCKING: c_int = 0;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
const LATENCY_US: c_uint = 50_000;

type Pcm = *mut c_void;
type FnOpen = unsafe extern "C" fn(*mut Pcm, *const c_char, c_int, c_int) -> c_int;
type FnSetParams = unsafe extern "C" fn(Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type FnWritei = unsafe extern "C" fn(Pcm, *const c_void, c_ulong) -> c_long;
type FnRecover = unsafe extern "C" fn(Pcm, c_int, c_int) -> c_int;
type FnPcm = unsafe extern "C" fn(Pcm) -> c_int;
type FnNonblock = unsafe extern "C" fn(Pcm, c_int) -> c_int;

pub(super) struct Output {
    open: FnOpen,
    set_params: FnSetParams,
    writei: FnWritei,
    recover: FnRecover,
    drain: FnPcm,
    close: FnPcm,
    nonblock: FnNonblock,
    reported: Cell<Option<c_int>>,
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
                crate::log::warn(&format!("ALSA yüklenemedi, sesler kapalı: {e}"));
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
            nonblock: symbol(&lib, "snd_pcm_nonblock")?,
            reported: Cell::new(None),
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
        let Some(pcm) = self.open_pcm(DEVICE) else {
            return;
        };
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

    fn open_pcm(&self, device: &CStr) -> Option<Pcm> {
        let mut pcm: Pcm = std::ptr::null_mut();
        // SAFETY: pcm yerel bir işaretçi; aygıt adı sıfır sonlu bir C dizgisi.
        let opened = unsafe {
            (self.open)(
                &mut pcm,
                device.as_ptr(),
                SND_PCM_STREAM_PLAYBACK,
                SND_PCM_NONBLOCK,
            )
        };
        if opened < 0 {
            self.report_open_failure(opened);
            return None;
        }
        if pcm.is_null() {
            self.report_open_failure(-libc::EINVAL);
            return None;
        }
        // SAFETY: pcm az önce açıldı; 0 aygıtı engelleyen yazma kipine alır.
        let blocking = unsafe { (self.nonblock)(pcm, SND_PCM_BLOCKING) };
        if blocking < 0 {
            self.report_open_failure(blocking);
            // SAFETY: pcm açık ve bundan sonra kullanılmaz.
            unsafe { (self.close)(pcm) };
            return None;
        }
        Some(pcm)
    }

    fn report_open_failure(&self, code: c_int) {
        if self.reported.replace(Some(code)) != Some(code) {
            crate::log::warn(&open_failure_text(code));
        }
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

fn open_failure_text(code: c_int) -> String {
    if code == -libc::EBUSY {
        return "ses aygıtı meşgul; uyarı sesi atlandı".to_owned();
    }
    format!(
        "ses aygıtı açılamadı, uyarı sesi atlandı: {}",
        std::io::Error::from_raw_os_error(-code)
    )
}
