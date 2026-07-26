use std::path::Path;

use anyhow::Context;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Averages interleaved multi-channel samples down to mono. A no-op (returns a copy)
/// when `channels <= 1`.
pub fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resamples mono `f32` samples from `input_rate` to `TARGET_SAMPLE_RATE` (16kHz),
/// the rate whisper.cpp requires. A no-op (returns a copy) when already at that rate.
pub fn resample_to_16k(samples: &[f32], input_rate: u32) -> anyhow::Result<Vec<f32>> {
    if input_rate == TARGET_SAMPLE_RATE || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = TARGET_SAMPLE_RATE as f64 / input_rate as f64;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, samples.len(), 1)
        .context("constructing the audio resampler")?;
    let waves_in = vec![samples.to_vec()];
    let mut waves_out = resampler.process(&waves_in, None).context("resampling audio to 16kHz")?;
    Ok(waves_out.remove(0))
}

/// Decodes an MP3/M4A file into its native-rate, native-channel-count interleaved
/// `f32` samples. Not unit-tested directly (needs a real encoded audio file) —
/// verified via the manual smoke test; `downmix_to_mono`/`resample_to_16k` above,
/// which this feeds into, carry the unit-testable logic.
fn decode_to_native_samples(path: &Path) -> anyhow::Result<(Vec<f32>, u32, usize)> {
    let src = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .with_context(|| format!("probing audio format for {}", path.display()))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .with_context(|| format!("no supported audio track in {}", path.display()))?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &dec_opts)
        .with_context(|| format!("unsupported codec in {}", path.display()))?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut collected: Vec<f32> = Vec::new();
    let mut rate = 0u32;
    let mut channels = 0usize;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err).context("reading an audio packet"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded: AudioBufferRef = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::DecodeError(_)) => continue,
            Err(err) => return Err(err).context("decoding an audio packet"),
        };
        if sample_buf.is_none() {
            let spec = *decoded.spec();
            rate = spec.rate;
            channels = spec.channels.count();
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        if let Some(buf) = &mut sample_buf {
            buf.copy_interleaved_ref(decoded);
            collected.extend_from_slice(buf.samples());
        }
    }

    anyhow::ensure!(channels > 0, "no audio samples decoded from {}", path.display());
    Ok((collected, rate, channels))
}

/// Decodes `path` (mp3/m4a) all the way to 16kHz mono `f32` PCM, ready for whisper-rs.
pub fn decode_to_pcm16k_mono(path: &Path) -> anyhow::Result<Vec<f32>> {
    let (samples, rate, channels) = decode_to_native_samples(path)?;
    let mono = downmix_to_mono(&samples, channels);
    resample_to_16k(&mono, rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_interleaved_channels() {
        // Two stereo frames: (L=1.0, R=3.0) and (L=0.0, R=0.0) -> mono [2.0, 0.0]
        let stereo = vec![1.0, 3.0, 0.0, 0.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![2.0, 0.0]);
    }

    #[test]
    fn downmix_is_a_noop_for_mono_input() {
        let mono = vec![0.1, 0.2, 0.3];
        assert_eq!(downmix_to_mono(&mono, 1), mono);
    }

    #[test]
    fn resample_is_a_noop_when_already_16k() {
        let samples = vec![0.1_f32; 100];
        let out = resample_to_16k(&samples, 16_000).unwrap();
        assert_eq!(out, samples);
    }

    #[test]
    fn resample_changes_sample_count_proportionally_to_rate() {
        // 1 second of 44.1kHz audio should resample to close to 1 second of 16kHz audio.
        let samples = vec![0.0_f32; 44_100];
        let out = resample_to_16k(&samples, 44_100).unwrap();
        let expected = 16_000;
        let tolerance = 1_000; // sinc resampling doesn't land on an exact sample count
        assert!(
            (out.len() as i64 - expected as i64).abs() < tolerance,
            "expected close to {expected} samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resample_handles_empty_input() {
        assert_eq!(resample_to_16k(&[], 44_100).unwrap(), Vec::<f32>::new());
    }
}
