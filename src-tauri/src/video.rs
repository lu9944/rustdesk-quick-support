// Screen capture (xcap) + H.264 encode (openh264) -> EncodedVideoFrame packets.
//
// IMPORTANT: DXGI output duplication on Windows is exclusive per output, so we
// must run exactly ONE capture thread process-wide. Multiple connections
// subscribe to that single producer via `set_subscriber`.
use anyhow::{anyhow, Result};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use openh264::encoder::{BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, UsageType};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use openh264::OpenH264API;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use xcap::Monitor;

pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub key: bool,
    pub pts: i64,
}

/// Current subscriber (the active connection's frame channel).
static SUBSCRIBER: Lazy<Mutex<Option<mpsc::Sender<EncodedFrame>>>> = Lazy::new(|| Mutex::new(None));
static STARTED: AtomicBool = AtomicBool::new(false);

/// Set the active frame subscriber (called by a connection on login).
pub fn set_subscriber(tx: mpsc::Sender<EncodedFrame>) {
    *SUBSCRIBER.lock().unwrap() = Some(tx);
}

pub fn clear_subscriber() {
    *SUBSCRIBER.lock().unwrap() = None;
}

/// Start the single global capture thread (idempotent).
pub fn start_global() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(capture_loop);
    info!("global video capture thread started");
}

fn capture_loop() {
    loop {
        match capture_once() {
            Ok(()) => break,
            Err(e) => {
                error!("video capture: {e}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn capture_once() -> Result<()> {
    let monitors = Monitor::all().map_err(|e| anyhow!("list monitors: {e}"))?;
    let monitor = monitors
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no monitor"))?;
    info!(
        "capturing monitor: {} {}x{}",
        monitor.name().unwrap_or_default(),
        monitor.width().unwrap_or(0),
        monitor.height().unwrap_or(0)
    );

    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u16)
        .unwrap_or(4);
    let config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .max_frame_rate(FrameRate::from_hz(15.0))
        .bitrate(BitRate::from_bps(2_000_000))
        .complexity(Complexity::Low)
        .num_threads(threads);
    let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)?;
    info!("video encoder: screen-content, low complexity, {threads} threads");

    let frame_dur = Duration::from_millis(100); // ~10 fps target
    let mut rgb8: Vec<u8> = Vec::new();
    let mut frame_count: u64 = 0;

    loop {
        let t0 = Instant::now();

        // Only capture+encode if there is a live subscriber; otherwise idle.
        let has_sub = SUBSCRIBER.lock().unwrap().is_some();
        if !has_sub {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        let img = match monitor.capture_image() {
            Ok(img) => img,
            Err(e) => {
                // Transient DXGI hiccups (e.g. mode switches): back off & retry.
                warn!("capture error: {e}");
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let t_cap = t0.elapsed();

        let (w, h) = (img.width(), img.height());
        let (we, he) = (w & !1, h & !1);
        if we == 0 || he == 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        let t1 = Instant::now();
        let raw = img.as_raw();
        rgb8.clear();
        rgb8.reserve((we as usize) * (he as usize) * 3);
        if we == w {
            for px in raw.chunks_exact(4) {
                rgb8.push(px[0]);
                rgb8.push(px[1]);
                rgb8.push(px[2]);
            }
        } else {
            let stride = w as usize * 4;
            for y in 0..he as usize {
                let row = &raw[y * stride..];
                for x in 0..we as usize {
                    let i = x * 4;
                    rgb8.push(row[i]);
                    rgb8.push(row[i + 1]);
                    rgb8.push(row[i + 2]);
                }
            }
        }
        let t_conv = t1.elapsed();

        let t2 = Instant::now();
        let slice = RgbSliceU8::new(&rgb8, (we as usize, he as usize));
        let yuv = YUVBuffer::from_rgb8_source(slice);
        let bitstream = encoder.encode(&yuv)?;
        let mut data = Vec::with_capacity(64 * 1024);
        bitstream.write_vec(&mut data);
        let key = matches!(bitstream.frame_type(), FrameType::I | FrameType::IDR);
        let t_enc = t2.elapsed();

        if frame_count % 30 == 0 {
            info!(
                "video #{frame_count} {we}x{he}: cap={}ms conv={}ms enc={}ms frame={}B",
                t_cap.as_millis(),
                t_conv.as_millis(),
                t_enc.as_millis(),
                data.len()
            );
        }
        frame_count += 1;

        let frame = EncodedFrame {
            data,
            key,
            pts: t0.elapsed().as_millis() as i64,
        };
        // Deliver to the current subscriber (non-blocking; drop if gone).
        let mut guard = SUBSCRIBER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            if tx.try_send(frame).is_err() {
                // subscriber channel full or closed; clear it
                *guard = None;
            }
        }

        let elapsed = t0.elapsed();
        if let Some(rem) = frame_dur.checked_sub(elapsed) {
            std::thread::sleep(rem);
        }
    }
}
