use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
use windows::core::PCWSTR;

pub(super) struct Output;

impl Output {
    pub(super) fn open() -> Option<Self> {
        Some(Self)
    }

    pub(super) fn play(&self, wav: &[u8]) {
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
