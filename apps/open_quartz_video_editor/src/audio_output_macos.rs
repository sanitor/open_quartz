use super::*;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BUFFER_COUNT: usize = 4;

pub(super) fn play(
    mut mix: AudioMix,
    start: u64,
    end: u64,
    clock: &AudioClock,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let (send, receive) = mpsc::sync_channel(BUFFER_COUNT);
    let queue = Queue::new(send)?;
    let mut next = start;
    let mut queued = 0;
    let mut played = 0;
    for _ in 0..BUFFER_COUNT {
        if next == end {
            break;
        }
        let buffer = queue.allocate()?;
        next = queue.enqueue(buffer, &mut mix, next, end, clock)?;
        queued += 1;
    }
    while !clock.video_ready.load(Ordering::Acquire) {
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        thread::park_timeout(Duration::from_millis(2));
    }
    check(unsafe { AudioQueueStart(queue.raw, null()) }, "start")?;
    let mut last_progress = Instant::now();
    let mut draining = next == end;
    if draining {
        check(unsafe { AudioQueueStop(queue.raw, false) }, "drain")?;
    }
    while !cancel.load(Ordering::Acquire) {
        if let Some(sample) = queue.current_sample()? {
            let sample = sample.min(next - start);
            if sample > played {
                last_progress = Instant::now();
                played = sample;
                clock.played.store(sample, Ordering::Release);
            }
        }
        while let Ok(buffer) = receive.try_recv() {
            queued -= 1;
            if next < end {
                next = queue.enqueue(buffer as *mut AudioBuffer, &mut mix, next, end, clock)?;
                queued += 1;
            }
        }
        if next == end && !draining {
            check(unsafe { AudioQueueStop(queue.raw, false) }, "drain")?;
            draining = true;
        }
        if draining && queued == 0 && !queue.is_running()? {
            clock.played.store(end - start, Ordering::Release);
            clock.peak.store(0, Ordering::Relaxed);
            for peak in &clock.track_peaks {
                peak.store(0, Ordering::Relaxed);
            }
            clock.finished.store(true, Ordering::Release);
            return Ok(());
        }
        if last_progress.elapsed() > Duration::from_secs(3) {
            return Err("Audio output stopped advancing".into());
        }
        thread::park_timeout(Duration::from_millis(2));
    }
    Ok(())
}

// Native AudioQueue types follow the AudioToolbox/CoreAudioTypes C ABI.
type Handle = *mut c_void;
#[repr(C)]
struct StreamFormat {
    sample_rate: f64,
    id: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits_per_channel: u32,
    reserved: u32,
}
#[repr(C)]
struct AudioBuffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user_data: *mut c_void,
    description_capacity: u32,
    descriptions: *mut c_void,
    description_count: u32,
}
#[repr(C)]
#[derive(Default)]
struct AudioTimeStamp {
    sample_time: f64,
    host_time: u64,
    rate_scalar: f64,
    word_clock: u64,
    smpte: [u32; 6],
    flags: u32,
    reserved: u32,
}
#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        format: *const StreamFormat,
        callback: unsafe extern "C" fn(Handle, Handle, *mut AudioBuffer),
        user: Handle,
        run_loop: *const c_void,
        mode: *const c_void,
        flags: u32,
        output: *mut Handle,
    ) -> i32;
    fn AudioQueueAllocateBuffer(queue: Handle, bytes: u32, buffer: *mut *mut AudioBuffer) -> i32;
    fn AudioQueueEnqueueBuffer(
        queue: Handle,
        buffer: *mut AudioBuffer,
        packet_count: u32,
        packets: *const c_void,
    ) -> i32;
    fn AudioQueueStart(queue: Handle, start_time: *const AudioTimeStamp) -> i32;
    fn AudioQueueStop(queue: Handle, immediate: bool) -> i32;
    fn AudioQueueDispose(queue: Handle, immediate: bool) -> i32;
    fn AudioQueueCreateTimeline(queue: Handle, timeline: *mut Handle) -> i32;
    fn AudioQueueDisposeTimeline(queue: Handle, timeline: Handle) -> i32;
    fn AudioQueueGetCurrentTime(
        queue: Handle,
        timeline: Handle,
        time: *mut AudioTimeStamp,
        discontinuity: *mut u8,
    ) -> i32;
    fn AudioQueueGetProperty(queue: Handle, property: u32, value: Handle, size: *mut u32) -> i32;
}

struct Queue {
    raw: Handle,
    timeline: Handle,
    callback_owner: Option<Box<mpsc::SyncSender<usize>>>,
}

impl Queue {
    fn new(send: mpsc::SyncSender<usize>) -> Result<Self, String> {
        let mut send = Box::new(send);
        let format = StreamFormat {
            sample_rate: SAMPLE_RATE as f64,
            id: u32::from_be_bytes(*b"lpcm"),
            flags: 1 | 8,
            bytes_per_packet: BYTES as u32,
            frames_per_packet: 1,
            bytes_per_frame: BYTES as u32,
            channels: 2,
            bits_per_channel: 32,
            reserved: 0,
        };
        let mut raw = null_mut();
        let user = (&mut *send as *mut mpsc::SyncSender<usize>).cast::<c_void>();
        check(
            unsafe { AudioQueueNewOutput(&format, consumed, user, null(), null(), 0, &mut raw) },
            "open",
        )?;
        let mut queue = Self {
            raw,
            timeline: null_mut(),
            callback_owner: Some(send),
        };
        check(
            unsafe { AudioQueueCreateTimeline(raw, &mut queue.timeline) },
            "create clock",
        )?;
        Ok(queue)
    }

    fn allocate(&self) -> Result<*mut AudioBuffer, String> {
        let mut buffer = null_mut();
        check(
            unsafe { AudioQueueAllocateBuffer(self.raw, (BLOCK * BYTES) as u32, &mut buffer) },
            "allocate buffer",
        )?;
        Ok(buffer)
    }

    fn enqueue(
        &self,
        buffer: *mut AudioBuffer,
        mix: &mut AudioMix,
        start: u64,
        end: u64,
        clock: &AudioClock,
    ) -> Result<u64, String> {
        let count = (end - start).min(BLOCK as u64) as usize;
        // This buffer is either new or returned by the callback. The queue does
        // not read it again until it is enqueued.
        let samples =
            unsafe { std::slice::from_raw_parts_mut((*buffer).data.cast::<StereoSample>(), count) };
        mix.render(start, samples)
            .map_err(|error| error.to_string())?;
        clock.update_meters(mix);
        unsafe {
            (*buffer).size = (count * BYTES) as u32;
        }
        check(
            unsafe { AudioQueueEnqueueBuffer(self.raw, buffer, 0, null()) },
            "enqueue",
        )?;
        Ok(start + count as u64)
    }

    fn current_sample(&self) -> Result<Option<u64>, String> {
        let mut time = AudioTimeStamp::default();
        let mut discontinuity = 0;
        let result = unsafe {
            AudioQueueGetCurrentTime(self.raw, self.timeline, &mut time, &mut discontinuity)
        };
        // A queue at startup or after its final buffer may temporarily have no clock.
        if result != 0 || time.flags & 1 == 0 {
            return Ok(None);
        }
        if discontinuity != 0 {
            return Err("Audio device clock changed during playback".into());
        }
        Ok(Some(time.sample_time.max(0.0) as u64))
    }

    fn is_running(&self) -> Result<bool, String> {
        let mut running = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        check(
            unsafe {
                AudioQueueGetProperty(
                    self.raw,
                    u32::from_be_bytes(*b"aqrn"),
                    (&mut running as *mut u32).cast(),
                    &mut size,
                )
            },
            "query",
        )?;
        Ok(running != 0)
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        unsafe {
            AudioQueueStop(self.raw, true);
            if !self.timeline.is_null() {
                AudioQueueDisposeTimeline(self.raw, self.timeline);
            }
            if AudioQueueDispose(self.raw, true) != 0 {
                // Retain the callback context if the device did not confirm disposal.
                if let Some(context) = self.callback_owner.take() {
                    let _ = Box::leak(context);
                }
            }
        }
    }
}

unsafe extern "C" fn consumed(user: Handle, _queue: Handle, buffer: *mut AudioBuffer) {
    let sender = unsafe { &*user.cast::<mpsc::SyncSender<usize>>() };
    let _ = sender.try_send(buffer as usize);
}

fn check(status: i32, operation: &str) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "Cannot {operation} audio output (CoreAudio {status})"
        ))
    }
}
