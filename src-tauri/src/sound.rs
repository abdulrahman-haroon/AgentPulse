//! Alert chimes, synthesized at runtime (no audio files to ship) and played with the OS sound API.
//! Toasts stay silent (the notification plugin's default), so each alert makes exactly one sound.

use crate::events::AlertKind;
use std::f32::consts::PI;

const RATE: u32 = 44_100;

/// (frequency Hz, start seconds) per note.
fn notes(kind: AlertKind) -> &'static [(f32, f32)] {
    match kind {
        // G5 → C6: a soft "finished".
        AlertKind::Done => &[(783.99, 0.0), (1046.50, 0.13)],
        // C6 E6 C6 E6: brighter and longer, so "your turn" is told apart without looking.
        AlertKind::NeedsYou => &[(1046.50, 0.0), (1318.51, 0.11), (1046.50, 0.22), (1318.51, 0.33)],
    }
}

/// 16-bit mono PCM WAV of a short bell-like chime.
pub fn wav(kind: AlertKind) -> Vec<u8> {
    let notes = notes(kind);
    let secs = notes.iter().map(|n| n.1).fold(0.0, f32::max) + 0.7;
    let mut pcm = vec![0f32; (secs * RATE as f32) as usize];
    for &(freq, start) in notes {
        let from = (start * RATE as f32) as usize;
        for (i, s) in pcm.iter_mut().enumerate().skip(from) {
            let t = (i - from) as f32 / RATE as f32;
            let envelope = (t / 0.005).min(1.0) * (-t * 7.5).exp(); // 5 ms attack, bell decay
            let w = 2.0 * PI * freq * t;
            *s += envelope * (w.sin() + 0.28 * (2.0 * w).sin() + 0.08 * (3.0 * w).sin());
        }
    }
    let peak = pcm.iter().fold(1e-6f32, |m, v| m.max(v.abs()));
    let gain = 0.45 / peak; // comfortable level; the system volume does the rest

    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&(((s * gain).clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(windows)]
pub fn play(kind: AlertKind) {
    use std::sync::OnceLock;
    static DONE: OnceLock<Vec<u8>> = OnceLock::new();
    static NEEDS_YOU: OnceLock<Vec<u8>> = OnceLock::new();
    let buf = match kind {
        AlertKind::Done => DONE.get_or_init(|| wav(kind)),
        AlertKind::NeedsYou => NEEDS_YOU.get_or_init(|| wav(kind)),
    };
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(sound: *const u8, module: *mut std::ffi::c_void, flags: u32) -> i32;
    }
    const SND_ASYNC: u32 = 0x1;
    const SND_NODEFAULT: u32 = 0x2;
    const SND_MEMORY: u32 = 0x4;
    // SAFETY: with SND_MEMORY the pointer is a complete WAV image; it lives in a static, so it
    // outlives the asynchronous playback.
    unsafe {
        PlaySoundW(buf.as_ptr(), std::ptr::null_mut(), SND_ASYNC | SND_MEMORY | SND_NODEFAULT);
    }
}

// shortcut: macOS/Linux builds aren't shipped yet; play through their sound APIs when they are.
#[cfg(not(windows))]
pub fn play(_kind: AlertKind) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_valid_wav_that_is_not_clipped() {
        for kind in [AlertKind::Done, AlertKind::NeedsYou] {
            let w = wav(kind);
            assert_eq!(&w[0..4], b"RIFF");
            assert_eq!(&w[8..16], b"WAVEfmt ");
            let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
            assert_eq!(w.len(), 44 + data_len);
            assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
            let peak = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
            assert!((10_000..i16::MAX as u16).contains(&peak), "{peak}");
        }
        assert!(wav(AlertKind::NeedsYou).len() > wav(AlertKind::Done).len()); // longer, so it's distinguishable
    }
}
