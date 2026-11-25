use super::*;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioClient, IAudioClock, IAudioRenderClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct RunningStream<'a>(&'a IAudioClient);
impl Drop for RunningStream<'_> {
    fn drop(&mut self) {
        let _ = unsafe { self.0.Stop() };
    }
}

pub(super) fn play(
    mut mix: AudioMix,
    start: u64,
    end: u64,
    clock: &AudioClock,
    cancel: &AtomicBool,
) -> Result<(), String> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|error| audio_error("initialize COM", error))?;
    let _apartment = Apartment;
    let devices: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| audio_error("enumerate endpoints", error))?;
    let endpoint = unsafe { devices.GetDefaultAudioEndpoint(eRender, eConsole) }
        .map_err(|error| audio_error("find the default playback endpoint", error))?;
    let client: IAudioClient = unsafe { endpoint.Activate(CLSCTX_ALL, None) }
        .map_err(|error| audio_error("activate the playback endpoint", error))?;
    let format = WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: 2,
        nSamplesPerSec: oq_media::AUDIO_RATE,
        nAvgBytesPerSec: oq_media::AUDIO_RATE * BYTES as u32,
        nBlockAlign: BYTES as u16,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            800_000,
            0,
            &format,
            None,
        )
    }
    .map_err(|error| audio_error("open the audio stream", error))?;
    let capacity = unsafe { client.GetBufferSize() }
        .map_err(|error| audio_error("read the buffer size", error))?;
    let render: IAudioRenderClient = unsafe { client.GetService() }
        .map_err(|error| audio_error("open the render buffer", error))?;
    let device_clock: IAudioClock = unsafe { client.GetService() }
        .map_err(|error| audio_error("open the device clock", error))?;
    let frequency = unsafe { device_clock.GetFrequency() }
        .map_err(|error| audio_error("read the clock frequency", error))?;
    if frequency == 0 {
        return Err("WASAPI returned an invalid audio clock frequency".into());
    }
    let mut scratch = [[0.0; 2]; BLOCK];
    let mut next = start;
    let mut available = capacity;
    while next < end && available > 0 {
        let count = (end - next).min(u64::from(available)).min(BLOCK as u64) as usize;
        enqueue(&render, &mut mix, next, &mut scratch[..count], clock)?;
        next += count as u64;
        available -= count as u32;
    }
    while !clock.video_ready.load(Ordering::Acquire) {
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        thread::park_timeout(Duration::from_millis(2));
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    unsafe { client.Start() }.map_err(|error| audio_error("start the stream", error))?;
    let _running = RunningStream(&client);
    let mut last_progress = Instant::now();
    let mut played = 0;
    while !cancel.load(Ordering::Acquire) {
        let mut position = 0;
        unsafe { device_clock.GetPosition(&mut position, None) }
            .map_err(|error| audio_error("read the playback position", error))?;
        let sample = (u128::from(position) * u128::from(SAMPLE_RATE) / u128::from(frequency))
            .min(u128::from(next - start)) as u64;
        if sample > played {
            last_progress = Instant::now();
            played = sample;
            clock.played.store(sample, Ordering::Release);
        }
        let padding = unsafe { client.GetCurrentPadding() }
            .map_err(|error| audio_error("read queued audio", error))?;
        if next == end && padding == 0 && played >= end - start {
            clock.peak.store(0, Ordering::Relaxed);
            for peak in &clock.track_peaks {
                peak.store(0, Ordering::Relaxed);
            }
            clock.finished.store(true, Ordering::Release);
            return Ok(());
        }
        let count = (end - next)
            .min(u64::from(capacity.saturating_sub(padding)))
            .min(BLOCK as u64) as usize;
        if count > 0 {
            enqueue(&render, &mut mix, next, &mut scratch[..count], clock)?;
            next += count as u64;
        }
        if last_progress.elapsed() > Duration::from_secs(3) {
            return Err("Audio output stopped advancing".into());
        }
        thread::park_timeout(Duration::from_millis(2));
    }
    Ok(())
}

fn enqueue(
    render: &IAudioRenderClient,
    mix: &mut AudioMix,
    start: u64,
    samples: &mut [StereoSample],
    clock: &AudioClock,
) -> Result<(), String> {
    mix.render(start, samples)
        .map_err(|error| error.to_string())?;
    clock.update_meters(mix);
    let count = samples.len() as u32;
    let buffer = unsafe { render.GetBuffer(count) }
        .map_err(|error| audio_error("reserve an audio buffer", error))?;
    unsafe {
        std::ptr::copy_nonoverlapping(samples.as_ptr().cast::<u8>(), buffer, samples.len() * BYTES);
        render.ReleaseBuffer(count, 0)
    }
    .map_err(|error| audio_error("submit an audio buffer", error))
}

fn audio_error(operation: &str, error: windows::core::Error) -> String {
    format!("WASAPI could not {operation}: {error}")
}
