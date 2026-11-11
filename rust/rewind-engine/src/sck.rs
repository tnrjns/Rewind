//! ScreenCaptureKit capture: the macOS recorder's alternative to AVFoundation,
//! used because only SCK can leave a window — our own status pill — out of the
//! frames.
//!
//! SCK is change-driven: the stream output fires only when the screen changed.
//! FFmpeg's `rawvideo` input carries no timestamps, so the writer thread here
//! paces the output itself: exactly `fps` tightly packed NV12 frames per second,
//! repeating the newest frame while the screen is idle. Without that ticker the
//! muxer would treat the missing frames as never having happened, and audio
//! would drift ahead of video.

use std::io::Write;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_core_video::{
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess, CVPixelBuffer,
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPlaneCount, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSError, NSObject};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput,
    SCStreamOutputType, SCWindow,
};

/// Title of the app's status pill window (`rewind-app`'s `ui::pill::TITLE`); SCK
/// excludes windows by exact title.
pub const EXCLUDED_WINDOW: &str = "Rewind status";

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

/// A running SCK stream, feeding a raw NV12 sink at a constant frame rate.
pub struct Capture {
    handle: Handle,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
    writer: Option<JoinHandle<()>>,
}

impl Capture {
    /// Start capturing the display and writing tightly packed NV12 frames to `sink`
    /// at a constant `fps`. `display` is a CGDirectDisplayID. If `exclude_title` is
    /// Some, any SCWindow with that exact title is excluded from the capture.
    pub fn start(
        sink: Box<dyn Write + Send>,
        display: u32,
        width: u32,
        height: u32,
        fps: u32,
        cursor: bool,
        exclude_title: Option<String>,
    ) -> Result<Capture, String> {
        if width == 0 || height == 0 || fps == 0 {
            return Err(format!(
                "sck: invalid capture geometry {width}x{height} at {fps} fps"
            ));
        }
        if width % 2 != 0 || height % 2 != 0 {
            return Err(format!(
                "sck: NV12 needs even dimensions, got {width}x{height}"
            ));
        }
        if fps > i32::MAX as u32 {
            return Err(format!("sck: frame rate {fps} is out of range"));
        }

        autoreleasepool(|_| {
            let content = shareable_content()?;

            let displays = unsafe { content.displays() };
            let target = displays
                .iter()
                .find(|d| unsafe { d.displayID() } == display);
            let Some(target) = target else {
                let ids: Vec<u32> = displays.iter().map(|d| unsafe { d.displayID() }).collect();
                return Err(format!(
                    "sck: display {display} not found; available display IDs: {ids:?}"
                ));
            };

            let mut excluded: Vec<Retained<SCWindow>> = Vec::new();
            if let Some(title) = exclude_title {
                let windows = unsafe { content.windows() };
                match windows
                    .iter()
                    .find(|w| unsafe { w.title() }.is_some_and(|t| t.to_string() == title))
                {
                    Some(window) => excluded.push(window.clone()),
                    // Not fatal: the pill may not exist yet when the engine starts.
                    None => {
                        eprintln!("sck: no window titled {title:?}; capturing without exclusions")
                    }
                }
            }
            let excluded = NSArray::from_retained_slice(&excluded);

            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &target,
                    &excluded,
                )
            };

            let config = unsafe { SCStreamConfiguration::new() };
            unsafe {
                config.setWidth(width as usize);
                config.setHeight(height as usize);
                config.setPixelFormat(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
                config.setMinimumFrameInterval(CMTime {
                    value: 1,
                    timescale: fps as i32,
                    flags: CMTimeFlags::Valid,
                    epoch: 0,
                });
                config.setQueueDepth(6);
                config.setShowsCursor(cursor);
            }

            let slot = FrameSlot::new();
            let output = Output::new(slot.clone(), width as usize, height as usize);
            let queue = DispatchQueue::new("rewind.sck", None);

            let stream = unsafe {
                SCStream::initWithFilter_configuration_delegate(
                    SCStream::alloc(),
                    &filter,
                    &config,
                    None,
                )
            };
            unsafe {
                stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        ProtocolObject::from_ref(&*output),
                        SCStreamOutputType::Screen,
                        Some(&queue),
                    )
                    .map_err(|error| {
                        format!("sck: addStreamOutput failed: {}", error_text(&error))
                    })?;
            }

            start_stream(&stream)?;

            let stop = Arc::new(AtomicBool::new(false));
            let writer = {
                let slot = slot.clone();
                let stop = stop.clone();
                thread::spawn(move || {
                    autoreleasepool(|_| {
                        write_frames(sink, slot, stop, fps, width as usize, height as usize)
                    })
                })
            };

            Ok(Capture {
                handle: Handle {
                    stream,
                    _output: output,
                    _queue: queue,
                },
                slot,
                stop,
                writer: Some(writer),
            })
        })
    }

    /// Stop the stream and join the writer. Frames already queued are dropped.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.slot.wake();
        autoreleasepool(|_| unsafe {
            self.handle.stream.stopCaptureWithCompletionHandler(None);
        });
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

/// The stream outlives the `start` call; SCK has no `Send`-friendly handle type,
/// but the only thing ever done to these objects afterwards is `stopCapture`,
/// which the framework documents as callable from any thread.
struct Handle {
    stream: Retained<SCStream>,
    // The output object and its queue must stay alive while the stream runs.
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: see the comment on `Handle`.
unsafe impl Send for Handle {}

/// Ask SCK for the shareable content, blocking until its completion handler fires.
fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = mpsc::channel::<Result<usize, String>>();
    let block: RcBlock<dyn Fn(*mut SCShareableContent, *mut NSError)> = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            autoreleasepool(|_| {
                let result = if content.is_null() {
                    Err(error_message(error))
                } else {
                    // Retain before the callback returns, then pass the reference
                    // across the channel through its raw pointer; the receiver turns
                    // it back into an owned reference. On timeout it leaks, which is
                    // a pointer-sized cost on an already failed capture.
                    unsafe { Retained::retain(content) }
                        .map(|content| Retained::into_raw(content) as usize)
                        .ok_or_else(|| "sck: shareable content vanished".to_string())
                };
                let _ = tx.send(result);
            });
        },
    );

    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false,
            true,
            &block,
        );
    }

    match rx.recv_timeout(CALLBACK_TIMEOUT) {
        Ok(Ok(raw)) => unsafe { Retained::from_raw(raw as *mut SCShareableContent) }
            .ok_or_else(|| "sck: shareable content was null".to_string()),
        Ok(Err(message)) => Err(format!("sck: {message}")),
        Err(_) => Err("sck: timed out waiting for shareable content".to_string()),
    }
}

/// Start the stream and wait for SCK's verdict, so `start` cannot return success
/// for a stream that never began.
fn start_stream(stream: &SCStream) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let block: RcBlock<dyn Fn(*mut NSError)> = RcBlock::new(move |error: *mut NSError| {
        let result = if error.is_null() {
            Ok(())
        } else {
            Err(error_message(error))
        };
        let _ = tx.send(result);
    });

    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&block));
    }

    match rx.recv_timeout(CALLBACK_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(format!("sck: startCapture failed: {message}")),
        Err(_) => Err("sck: timed out starting capture".to_string()),
    }
}

/// Describes an NSError pointer handed over by SCK; null means unknown.
fn error_message(error: *mut NSError) -> String {
    if error.is_null() {
        return "unknown error".to_string();
    }
    // SAFETY: SCK hands us a live NSError pointer for the duration of the call.
    error_text(unsafe { &*error })
}

fn error_text(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

/// Latest frame from the stream, shared between the sample callback and the writer.
struct FrameSlot {
    frame: Mutex<Option<Vec<u8>>>,
    ready: Condvar,
}

impl FrameSlot {
    fn new() -> Arc<FrameSlot> {
        Arc::new(FrameSlot {
            frame: Mutex::new(None),
            ready: Condvar::new(),
        })
    }

    /// Replace the pending frame: the writer only ever needs the newest one.
    fn put(&self, frame: Vec<u8>) {
        let mut slot = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(frame);
        drop(slot);
        self.ready.notify_one();
    }

    fn take(&self) -> Option<Vec<u8>> {
        self.frame.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    fn wait_for_frame(&self, stop: &AtomicBool) -> Option<Vec<u8>> {
        let mut slot = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(frame) = slot.take() {
                return Some(frame);
            }
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let (next, _) = self
                .ready
                .wait_timeout(slot, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
            slot = next;
        }
    }

    fn wake(&self) {
        self.ready.notify_all();
    }
}

struct OutputIvars {
    slot: Arc<FrameSlot>,
    width: usize,
    height: usize,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Output` owns its ivars.
    #[unsafe(super = NSObject)]
    #[ivars = OutputIvars]
    struct Output;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for Output {}

    // SAFETY: the signature matches SCStreamOutput's optional callback.
    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output_sample_buffer(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            r#type: SCStreamOutputType,
        ) {
            if r#type != SCStreamOutputType::Screen {
                return;
            }
            // SAFETY: the callback only runs while the stream is alive.
            let Some(image) = (unsafe { sample_buffer.image_buffer() }) else {
                return;
            };
            if let Some(frame) = copy_nv12(&image, self.ivars().width, self.ivars().height) {
                self.ivars().slot.put(frame);
            }
        }
    }
);

impl Output {
    fn new(slot: Arc<FrameSlot>, width: usize, height: usize) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars {
            slot,
            width,
            height,
        });
        // SAFETY: `init` on NSObject has no extra requirements.
        unsafe { msg_send![super(this), init] }
    }
}

/// Copy both NV12 planes into one tight `width * height * 3 / 2` buffer, dropping
/// the row padding IOSurfaces carry.
fn copy_nv12(image: &CVPixelBuffer, width: usize, height: usize) -> Option<Vec<u8>> {
    if CVPixelBufferGetPlaneCount(image) < 2 {
        return None;
    }
    let luma_rows = height;
    let chroma_rows = height / 2;
    let y_stride = CVPixelBufferGetBytesPerRowOfPlane(image, 0);
    let uv_stride = CVPixelBufferGetBytesPerRowOfPlane(image, 1);
    if y_stride < width
        || uv_stride < width
        || CVPixelBufferGetHeightOfPlane(image, 0) < luma_rows
        || CVPixelBufferGetHeightOfPlane(image, 1) < chroma_rows
    {
        return None;
    }

    // The plane base addresses are only valid once the buffer is locked.
    if unsafe { CVPixelBufferLockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly) }
        != kCVReturnSuccess
    {
        return None;
    }
    let y_base = CVPixelBufferGetBaseAddressOfPlane(image, 0).cast::<u8>();
    let uv_base = CVPixelBufferGetBaseAddressOfPlane(image, 1).cast::<u8>();
    if y_base.is_null() || uv_base.is_null() {
        // SAFETY: the buffer was locked above and is not accessed again.
        unsafe {
            CVPixelBufferUnlockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly);
        }
        return None;
    }

    let mut out = vec![0u8; width * height * 3 / 2];
    let (luma, chroma) = out.split_at_mut(width * luma_rows);
    for row in 0..luma_rows {
        // SAFETY: both ranges are within the locked plane for this row.
        unsafe {
            ptr::copy_nonoverlapping(
                y_base.add(row * y_stride),
                luma[row * width..].as_mut_ptr(),
                width,
            );
        }
    }
    for row in 0..chroma_rows {
        // SAFETY: same as above, for the interleaved UV plane.
        unsafe {
            ptr::copy_nonoverlapping(
                uv_base.add(row * uv_stride),
                chroma[row * width..].as_mut_ptr(),
                width,
            );
        }
    }

    // SAFETY: the buffer was locked above and is no longer accessed.
    unsafe {
        CVPixelBufferUnlockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly);
    }
    Some(out)
}

/// Constant-frame-rate writer: one frame per tick, repeating the last frame when
/// SCK had nothing new to deliver.
fn write_frames(
    mut sink: Box<dyn Write + Send>,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
    fps: u32,
    width: usize,
    height: usize,
) {
    let period = Duration::from_secs_f64(1.0 / fps as f64);
    let mut current = match slot.wait_for_frame(&stop) {
        Some(frame) => frame,
        None => return,
    };

    let expected = width * height * 3 / 2;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        next += period;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now.duration_since(next) > period {
            // Fell far behind (e.g. a stalled write): resume at the current time
            // instead of bursting to catch up and inflating the frame count.
            next = now;
        }

        // Take whatever arrived since the last tick; repeat `current` otherwise.
        if let Some(frame) = slot.take() {
            current = frame;
        }
        if current.len() != expected {
            continue;
        }
        if sink.write_all(&current).is_err() {
            stop.store(true, Ordering::Relaxed);
            return;
        }
    }
}
