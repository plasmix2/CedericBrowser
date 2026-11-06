//! Video on Android through the NDK's `AMediaExtractor` and `AMediaCodec` (libmediandk).
//!
//! MediaCodec picks the phone's hardware decoder for the track's format (H.264, HEVC, VP9 and
//! AV1, as the phone supports them). Decoded pictures come back as YUV 4:2:0, planar (I420) or
//! semi-planar (NV12), and are converted to RGBA on the CPU.
//!
//! The extractor reads from a file descriptor, so the video's bytes go into an anonymous
//! in-memory file (`memfd_create`).

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::null_mut;

use crate::media::Frame;

type media_status_t = i32;
type Ptr = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct AMediaCodecBufferInfo {
    offset: i32,
    size: i32,
    presentation_time_us: i64,
    flags: u32,
}

const BUFFER_FLAG_END_OF_STREAM: u32 = 4;
const INFO_OUTPUT_FORMAT_CHANGED: isize = -2;
/// How long one dequeue call may wait, in microseconds.
const WAIT_US: i64 = 10_000;

#[link(name = "mediandk")]
extern "C" {
    fn AMediaExtractor_new() -> Ptr;
    fn AMediaExtractor_delete(ex: Ptr) -> media_status_t;
    fn AMediaExtractor_setDataSourceFd(ex: Ptr, fd: i32, offset: i64, length: i64) -> media_status_t;
    fn AMediaExtractor_getTrackCount(ex: Ptr) -> usize;
    fn AMediaExtractor_getTrackFormat(ex: Ptr, index: usize) -> Ptr;
    fn AMediaExtractor_selectTrack(ex: Ptr, index: usize) -> media_status_t;
    fn AMediaExtractor_readSampleData(ex: Ptr, buffer: *mut u8, capacity: usize) -> isize;
    fn AMediaExtractor_getSampleTime(ex: Ptr) -> i64;
    fn AMediaExtractor_advance(ex: Ptr) -> bool;
    fn AMediaExtractor_seekTo(ex: Ptr, time_us: i64, mode: i32) -> media_status_t;
    fn AMediaFormat_delete(f: Ptr) -> media_status_t;
    fn AMediaFormat_getString(f: Ptr, name: *const c_char, out: *mut *const c_char) -> bool;
    fn AMediaFormat_getInt32(f: Ptr, name: *const c_char, out: *mut i32) -> bool;
    fn AMediaFormat_getInt64(f: Ptr, name: *const c_char, out: *mut i64) -> bool;
    fn AMediaCodec_createDecoderByType(mime: *const c_char) -> Ptr;
    fn AMediaCodec_configure(codec: Ptr, format: Ptr, surface: Ptr, crypto: Ptr, flags: u32) -> media_status_t;
    fn AMediaCodec_start(codec: Ptr) -> media_status_t;
    fn AMediaCodec_stop(codec: Ptr) -> media_status_t;
    fn AMediaCodec_delete(codec: Ptr) -> media_status_t;
    fn AMediaCodec_flush(codec: Ptr) -> media_status_t;
    fn AMediaCodec_dequeueInputBuffer(codec: Ptr, timeout_us: i64) -> isize;
    fn AMediaCodec_getInputBuffer(codec: Ptr, index: usize, size: *mut usize) -> *mut u8;
    fn AMediaCodec_queueInputBuffer(codec: Ptr, index: usize, offset: i64, size: usize, time_us: u64, flags: u32) -> media_status_t;
    fn AMediaCodec_dequeueOutputBuffer(codec: Ptr, info: *mut AMediaCodecBufferInfo, timeout_us: i64) -> isize;
    fn AMediaCodec_getOutputBuffer(codec: Ptr, index: usize, size: *mut usize) -> *mut u8;
    fn AMediaCodec_getOutputFormat(codec: Ptr) -> Ptr;
    fn AMediaCodec_releaseOutputBuffer(codec: Ptr, index: usize, render: bool) -> media_status_t;
}

extern "C" {
    fn syscall(number: i64, ...) -> i64;
    fn write(fd: i32, buf: *const c_void, count: usize) -> isize;
    fn close(fd: i32) -> i32;
}

/// `memfd_create` on aarch64 Linux.
const SYS_MEMFD_CREATE: i64 = 279;

fn key(name: &str) -> CString {
    CString::new(name).expect("format keys have no NUL")
}

/// The layout of decoded pictures, from the codec's output format.
#[derive(Clone, Copy, Debug)]
struct Layout {
    width: usize,
    height: usize,
    stride: usize,
    slice_height: usize,
    /// 19 is I420 (planar). 21 and the flexible 0x7f420888 come out NV12 on the decoders we
    /// target.
    color: i32,
}

pub struct Decoder {
    extractor: Ptr,
    codec: Ptr,
    fd: i32,
    layout: Option<Layout>,
    input_done: bool,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub hardware: bool,
}

// SAFETY: the extractor and codec are used from one thread at a time, through &mut self.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        // SAFETY: NDK calls with valid arguments. Every handle is freed in Drop, or here on error.
        unsafe {
            let name = key("anvl-video");
            let fd = syscall(SYS_MEMFD_CREATE, name.as_ptr(), 0u32) as i32;
            if fd < 0 {
                return Err("could not hold the video in memory".into());
            }
            let mut written = 0;
            while written < bytes.len() {
                let n = write(fd, bytes[written..].as_ptr() as *const c_void, bytes.len() - written);
                if n <= 0 {
                    close(fd);
                    return Err("could not hold the video in memory".into());
                }
                written += n as usize;
            }
            let extractor = AMediaExtractor_new();
            let mut d = Decoder {
                extractor,
                codec: null_mut(),
                fd,
                layout: None,
                input_done: false,
                width: 0,
                height: 0,
                duration: 0.0,
                hardware: true,
            };
            if AMediaExtractor_setDataSourceFd(extractor, fd, 0, bytes.len() as i64) != 0 {
                return Err("the file is not a video Android can read".into());
            }
            let (mime_key, w_key, h_key, dur_key) = (key("mime"), key("width"), key("height"), key("durationUs"));
            for i in 0..AMediaExtractor_getTrackCount(extractor) {
                let format = AMediaExtractor_getTrackFormat(extractor, i);
                let mut mime: *const c_char = std::ptr::null();
                let is_video = AMediaFormat_getString(format, mime_key.as_ptr(), &mut mime)
                    && !mime.is_null()
                    && CStr::from_ptr(mime).to_bytes().starts_with(b"video/");
                if !is_video {
                    AMediaFormat_delete(format);
                    continue;
                }
                let (mut w, mut h, mut dur) = (0i32, 0i32, 0i64);
                AMediaFormat_getInt32(format, w_key.as_ptr(), &mut w);
                AMediaFormat_getInt32(format, h_key.as_ptr(), &mut h);
                AMediaFormat_getInt64(format, dur_key.as_ptr(), &mut dur);
                d.width = w.max(0) as u32;
                d.height = h.max(0) as u32;
                d.duration = dur as f64 / 1e6;
                AMediaExtractor_selectTrack(extractor, i);
                d.codec = AMediaCodec_createDecoderByType(mime);
                let ok = !d.codec.is_null()
                    && AMediaCodec_configure(d.codec, format, null_mut(), null_mut(), 0) == 0
                    && AMediaCodec_start(d.codec) == 0;
                AMediaFormat_delete(format);
                if !ok {
                    return Err("this phone has no decoder for the video's format".into());
                }
                return Ok(d);
            }
            Err("the file has no video track".into())
        }
    }

    /// Feeds compressed samples in and returns the next decoded picture and its time.
    pub fn next(&mut self) -> Result<Option<(f64, Frame)>, String> {
        // SAFETY: NDK calls on handles this decoder owns. Buffers are read within the sizes the
        // codec reports, and released right after.
        unsafe {
            for _ in 0..1000 {
                if !self.input_done {
                    let index = AMediaCodec_dequeueInputBuffer(self.codec, WAIT_US);
                    if index >= 0 {
                        let mut cap = 0usize;
                        let buf = AMediaCodec_getInputBuffer(self.codec, index as usize, &mut cap);
                        let n = AMediaExtractor_readSampleData(self.extractor, buf, cap);
                        if n < 0 {
                            AMediaCodec_queueInputBuffer(self.codec, index as usize, 0, 0, 0, BUFFER_FLAG_END_OF_STREAM);
                            self.input_done = true;
                        } else {
                            let t = AMediaExtractor_getSampleTime(self.extractor).max(0) as u64;
                            AMediaCodec_queueInputBuffer(self.codec, index as usize, 0, n as usize, t, 0);
                            AMediaExtractor_advance(self.extractor);
                        }
                    }
                }
                let mut info = AMediaCodecBufferInfo::default();
                let index = AMediaCodec_dequeueOutputBuffer(self.codec, &mut info, WAIT_US);
                if index == INFO_OUTPUT_FORMAT_CHANGED {
                    self.read_layout();
                    continue;
                }
                if index < 0 {
                    continue;
                }
                if self.layout.is_none() {
                    self.read_layout();
                }
                let mut size = 0usize;
                let data = AMediaCodec_getOutputBuffer(self.codec, index as usize, &mut size);
                let frame = match (data.is_null(), self.layout) {
                    (false, Some(l)) if info.size > 0 => {
                        let bytes = std::slice::from_raw_parts(data.add(info.offset.max(0) as usize), (info.size as usize).min(size));
                        Some(yuv_to_rgba(bytes, l))
                    }
                    _ => None,
                };
                AMediaCodec_releaseOutputBuffer(self.codec, index as usize, false);
                if info.flags & BUFFER_FLAG_END_OF_STREAM != 0 && frame.is_none() {
                    return Ok(None);
                }
                if let Some(frame) = frame {
                    let frame = frame?;
                    self.width = frame.width;
                    self.height = frame.height;
                    return Ok(Some((info.presentation_time_us as f64 / 1e6, frame)));
                }
            }
            Err("the decoder stopped producing pictures".into())
        }
    }

    unsafe fn read_layout(&mut self) {
        let format = AMediaCodec_getOutputFormat(self.codec);
        if format.is_null() {
            return;
        }
        let get = |name: &str| {
            let mut v = 0i32;
            AMediaFormat_getInt32(format, key(name).as_ptr(), &mut v).then_some(v)
        };
        let width = get("width").unwrap_or(self.width as i32).max(1) as usize;
        let height = get("height").unwrap_or(self.height as i32).max(1) as usize;
        let stride = get("stride").filter(|&s| s > 0).map_or(width, |s| s as usize);
        let slice_height = get("slice-height").filter(|&s| s > 0).map_or(height, |s| s as usize);
        let color = get("color-format").unwrap_or(21);
        // The visible area, when the decoder pads its pictures.
        let (cw, ch) = match (get("crop-left"), get("crop-right"), get("crop-top"), get("crop-bottom")) {
            (Some(l), Some(r), Some(t), Some(b)) => ((r - l + 1) as usize, (b - t + 1) as usize),
            _ => (width, height),
        };
        AMediaFormat_delete(format);
        self.layout = Some(Layout { width: cw.min(width), height: ch.min(height), stride, slice_height, color });
    }

    pub fn rewind(&mut self) -> Result<(), String> {
        // SAFETY: seek and flush on owned handles.
        unsafe {
            AMediaExtractor_seekTo(self.extractor, 0, 2);
            AMediaCodec_flush(self.codec);
        }
        self.input_done = false;
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: each handle is freed once, here.
        unsafe {
            if !self.codec.is_null() {
                AMediaCodec_stop(self.codec);
                AMediaCodec_delete(self.codec);
            }
            AMediaExtractor_delete(self.extractor);
            close(self.fd);
        }
    }
}

/// YUV 4:2:0 (I420 or NV12) to RGBA with BT.601 limited-range coefficients.
fn yuv_to_rgba(bytes: &[u8], l: Layout) -> Result<Frame, String> {
    let y_size = l.stride * l.slice_height;
    let planar = l.color == 19;
    let need = y_size + if planar { 2 * (l.stride / 2) * (l.slice_height / 2) } else { l.stride * (l.slice_height / 2) };
    if bytes.len() < need.min(y_size + 1) {
        return Err("a decoded picture came out short".into());
    }
    let at = |i: usize| bytes.get(i).copied().unwrap_or(128) as f32;
    let mut rgba = Vec::with_capacity(l.width * l.height * 4);
    for y in 0..l.height {
        for x in 0..l.width {
            let luma = at(y * l.stride + x);
            let (u, v) = if planar {
                let c = (y / 2) * (l.stride / 2) + x / 2;
                (at(y_size + c), at(y_size + (l.stride / 2) * (l.slice_height / 2) + c))
            } else {
                let c = y_size + (y / 2) * l.stride + (x / 2) * 2;
                (at(c), at(c + 1))
            };
            let (c, d, e) = (1.164 * (luma - 16.0), u - 128.0, v - 128.0);
            let px = |f: f32| f.round().clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[px(c + 1.596 * e), px(c - 0.392 * d - 0.813 * e), px(c + 2.017 * d), 255]);
        }
    }
    Ok(Frame { width: l.width as u32, height: l.height as u32, rgba })
}
