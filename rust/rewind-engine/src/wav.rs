//! Minimal 16-bit PCM WAV writer.
//!
//! Clip audio is buffered as raw s16le and muxed by FFmpeg, which needs a container
//! to know the rate and channel count. One 44-byte header is all that takes.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// Write `pcm` (interleaved s16le) as a canonical 44-byte-header WAV file.
pub fn write_wav(path: &Path, pcm: &[u8], rate: u32, channels: u32) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let channels = channels.max(1);
    let block_align = 2 * channels as u16;
    let byte_rate = rate * u32::from(block_align);
    // RIFF is 32-bit; truncate rather than emit a corrupt header.
    let data_len = pcm.len().min(u32::MAX as usize) as u32;

    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36u32.saturating_add(data_len)).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&(channels as u16).to_le_bytes());
    header[24..28].copy_from_slice(&rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_len.to_le_bytes());

    let mut file = File::create(path)?;
    file.write_all(&header)?;
    file.write_all(&pcm[..data_len as usize])?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_describes_the_pcm() {
        let dir = std::env::temp_dir().join(format!("rewind-wav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        let pcm = vec![0u8; 48_000 * 4]; // 0.5 s stereo at 48 kHz
        write_wav(&path, &pcm, 48_000, 2).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize,
            pcm.len()
        );
        assert_eq!(bytes.len(), 44 + pcm.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mono_uses_the_right_block_align() {
        let dir = std::env::temp_dir().join(format!("rewind-wav-mono-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.wav");
        write_wav(&path, &[0u8; 100], 44_100, 1).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(u16::from_le_bytes(bytes[32..34].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            88_200
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
