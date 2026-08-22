use std::fs::File;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub fn convert_to_ogg_opus(input_path: &str) -> Result<Vec<u8>, String> {
    let (pcm_i16, _rate) = decode_audio_to_i16_mono(input_path)?;
    ogg_opus::encode::<48000, 1>(&pcm_i16).map_err(|e| format!("ogg-opus encode: {:?}", e))
}

fn decode_audio_to_i16_mono(path: &str) -> Result<(Vec<i16>, u32), String> {
    let file = File::open(path).map_err(|e| format!("open audio file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();
    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .map_err(|e| format!("probe format: {e}"))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio track found")?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("unknown sample rate")?;
    let n_channels = track
        .codec_params
        .channels
        .ok_or("unknown channel count")?
        .count();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .map_err(|e| format!("create decoder: {e}"))?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut pcm_f32: Vec<f32> = Vec::new();

    while let Ok(packet) = format.next_packet() {
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("decode packet: {e}"))?;
        if sample_buf.is_none() {
            let spec = *decoded.spec();
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        if let Some(buf) = sample_buf.as_mut() {
            buf.copy_planar_ref(decoded);
            pcm_f32.extend_from_slice(buf.samples());
        }
    }

    // Downmix to mono
    let mono: Vec<f32> = if n_channels > 1 {
        pcm_f32
            .chunks_exact(n_channels)
            .map(|chunk| chunk.iter().sum::<f32>() / n_channels as f32)
            .collect()
    } else {
        pcm_f32
    };

    // Resample to 48000 if needed
    let final_pcm = if sample_rate != 48000 {
        resample_f32_to_48k(&mono, sample_rate)?
    } else {
        mono
    };

    // Convert f32 [-1.0, 1.0] to i16
    let pcm_i16: Vec<i16> = final_pcm
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();

    Ok((pcm_i16, 48000))
}

fn resample_f32_to_48k(input: &[f32], orig_rate: u32) -> Result<Vec<f32>, String> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let ratio = 48000.0 / orig_rate as f64;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(ratio, 0.95, params, input.len().min(8192), 1)
        .map_err(|e| format!("resampler create: {:?}", e))?;

    let waves = vec![input.to_vec()];
    let output = resampler
        .process(&waves, None)
        .map_err(|e| format!("resample process: {:?}", e))?;
    Ok(output.into_iter().next().unwrap_or_default())
}
