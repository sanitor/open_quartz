use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn emit_perf(message: std::fmt::Arguments<'_>) {
    log::info!(target: "open_quartz::perf", "{message}");
    if std::env::var_os("OPEN_QUARTZ_NATIVE_VIDEO_BENCH").is_some()
        || std::env::var_os("OPEN_QUARTZ_PERF_LOG").is_some()
    {
        println!("[oq:perf] {message}");
    }
}

pub(super) struct NativeRenderPerf {
    window_started: Instant,
    frames: u64,
    upload_video: Duration,
    engine_plan: Duration,
    execute_gpu: Duration,
    total: Duration,
    max_total: Duration,
}

impl NativeRenderPerf {
    pub(super) fn new() -> Self {
        Self {
            window_started: Instant::now(),
            frames: 0,
            upload_video: Duration::ZERO,
            engine_plan: Duration::ZERO,
            execute_gpu: Duration::ZERO,
            total: Duration::ZERO,
            max_total: Duration::ZERO,
        }
    }

    pub(super) fn record(
        &mut self,
        upload_video: Duration,
        engine_plan: Duration,
        execute_gpu: Duration,
        total: Duration,
        decoded_frames: u64,
        uploaded_frames: u64,
    ) {
        if self.frames == 0 {
            self.window_started = Instant::now();
        }
        self.frames += 1;
        self.upload_video += upload_video;
        self.engine_plan += engine_plan;
        self.execute_gpu += execute_gpu;
        self.total += total;
        self.max_total = self.max_total.max(total);
        let window = self.window_started.elapsed();
        if window < Duration::from_secs(1) {
            return;
        }
        let frames = self.frames.max(1) as f64;
        emit_perf(format_args!(
            "native-render fps={:.1} avg_ms={{upload_video:{:.3},engine_plan:{:.3},execute_gpu:{:.3},total:{:.3}}} max_total_ms={:.3} video={{decoded:{decoded_frames},uploaded:{uploaded_frames}}}",
            self.frames as f64 / window.as_secs_f64(),
            self.upload_video.as_secs_f64() * 1000.0 / frames,
            self.engine_plan.as_secs_f64() * 1000.0 / frames,
            self.execute_gpu.as_secs_f64() * 1000.0 / frames,
            self.total.as_secs_f64() * 1000.0 / frames,
            self.max_total.as_secs_f64() * 1000.0,
        ));
        *self = Self::new();
    }
}
pub(super) fn utc_date_uniform(time: SystemTime) -> [f32; 4] {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = (seconds % 86_400) as f32;
    let (year, month, day) = civil_from_days(days);
    [year as f32, month as f32, day as f32, seconds_of_day]
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unix_time_to_wgsl_date_uniform() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(
            utc_date_uniform(UNIX_EPOCH + Duration::from_secs(86_400)),
            [1970.0, 1.0, 2.0, 0.0]
        );
    }

    #[test]
    fn date_uniform_handles_utc_rollover_and_gregorian_century_rules() {
        for (seconds, expected) in [
            (946_684_799, [1999.0, 12.0, 31.0, 86_399.0]),
            (946_684_800, [2000.0, 1.0, 1.0, 0.0]),
            (951_827_696, [2000.0, 2.0, 29.0, 45_296.0]),
            (951_868_800, [2000.0, 3.0, 1.0, 0.0]),
            (4_107_542_399, [2100.0, 2.0, 28.0, 86_399.0]),
            (4_107_542_400, [2100.0, 3.0, 1.0, 0.0]),
            (13_574_563_200, [2400.0, 2.0, 29.0, 0.0]),
        ] {
            assert_eq!(
                utc_date_uniform(UNIX_EPOCH + Duration::from_secs(seconds)),
                expected
            );
            assert_eq!(
                utc_date_uniform(
                    UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_nanos(999_999_999)
                ),
                expected
            );
        }
        assert_eq!(
            utc_date_uniform(UNIX_EPOCH - Duration::from_secs(1)),
            [1970.0, 1.0, 1.0, 0.0]
        );
    }
}
