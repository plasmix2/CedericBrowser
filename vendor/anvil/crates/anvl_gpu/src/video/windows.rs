//! Video on Windows through Media Foundation's source reader.
//!
//! The reader demuxes the file and runs the system's decoder, which is the GPU's hardware decoder
//! (DXVA) when a Direct3D 11 device can be made and handed to it, and Media Foundation's own
//! software decoder otherwise. Frames come out as 32-bit RGB in system memory. Codecs are what
//! Windows has installed: H.264 and MPEG-4 always, HEVC, VP9 and AV1 with their (often
//! preinstalled) extensions.
//!
//! The COM interfaces are called through their vtables, at slot numbers taken from the Windows
//! SDK headers, so no binding crate is needed. Libraries load with `raw-dylib`, so no import
//! libraries are needed either.

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::ptr::null_mut;

use crate::media::Frame;

type HRESULT = i32;
type Ptr = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct GUID(u32, u16, u16, [u8; 8]);

const GUID_NULL: GUID = GUID(0, 0, 0, [0; 8]);
const MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING: GUID = GUID(0xfb394f3d, 0xccf1, 0x42ee, [0xbb, 0xb3, 0xf9, 0xb8, 0x45, 0xd5, 0x68, 0x1d]);
const MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING: GUID =
    GUID(0x0f81da2c, 0xb537, 0x4672, [0xa8, 0xb2, 0xa6, 0x81, 0xb1, 0x73, 0x07, 0xa3]);
const MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS: GUID = GUID(0xa634a91c, 0x822b, 0x41b9, [0xa4, 0x94, 0x4d, 0xe4, 0x64, 0x36, 0x12, 0xb0]);
const MF_SOURCE_READER_D3D_MANAGER: GUID = GUID(0xec822da2, 0xe1e9, 0x4b29, [0xa0, 0xd8, 0x56, 0x3c, 0x71, 0x9f, 0x52, 0x69]);
const MF_MT_MAJOR_TYPE: GUID = GUID(0x48eba18e, 0xf8c9, 0x4687, [0xbf, 0x11, 0x0a, 0x74, 0xc9, 0xf9, 0x6a, 0x8f]);
const MF_MT_SUBTYPE: GUID = GUID(0xf7e34c9a, 0x42e8, 0x4714, [0xb7, 0x4b, 0xcb, 0x29, 0xd7, 0x2c, 0x35, 0xe5]);
const MF_MT_FRAME_SIZE: GUID = GUID(0x1652c33d, 0xd6b2, 0x4012, [0xb8, 0x34, 0x72, 0x03, 0x08, 0x49, 0xa3, 0x7d]);
const MF_MT_DEFAULT_STRIDE: GUID = GUID(0x644b4e48, 0x1e02, 0x4516, [0xb0, 0xeb, 0xc0, 0x1c, 0xa9, 0xd4, 0x9a, 0xc6]);
const MF_PD_DURATION: GUID = GUID(0x6c990d33, 0xbb8e, 0x477a, [0x85, 0x98, 0x0d, 0x5d, 0x96, 0xfc, 0xd8, 0x8a]);
const MF_MEDIA_TYPE_VIDEO: GUID = GUID(0x73646976, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
/// `MFVideoFormat_RGB32`: D3DFMT_X8R8G8B8 (22), stored as B, G, R, X bytes.
const MF_VIDEO_FORMAT_RGB32: GUID = GUID(22, 0x0000, 0x0010, [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
const IID_ID3D10_MULTITHREAD: GUID = GUID(0x9b7e4e00, 0x342c, 0x4106, [0xa1, 0x9f, 0x4f, 0x27, 0x04, 0xf6, 0x89, 0xf0]);

const MF_VERSION: u32 = 0x0002_0070;
const FIRST_VIDEO_STREAM: u32 = 0xffff_fffc;
const ALL_STREAMS: u32 = 0xffff_fffe;
const MEDIA_SOURCE: u32 = 0xffff_ffff;
const END_OF_STREAM: u32 = 0x2;
const VT_I8: u16 = 20;

/// A PROPVARIANT holding one 64-bit number: 24 bytes, the value at offset 8.
#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    value: i64,
    pad: u64,
}

#[link(name = "mfplat", kind = "raw-dylib")]
extern "system" {
    fn MFStartup(version: u32, flags: u32) -> HRESULT;
    fn MFCreateAttributes(attributes: *mut Ptr, size: u32) -> HRESULT;
    fn MFCreateMediaType(media_type: *mut Ptr) -> HRESULT;
    fn MFCreateMFByteStreamOnStream(stream: Ptr, byte_stream: *mut Ptr) -> HRESULT;
    fn MFCreateDXGIDeviceManager(token: *mut u32, manager: *mut Ptr) -> HRESULT;
}

#[link(name = "mfreadwrite", kind = "raw-dylib")]
extern "system" {
    fn MFCreateSourceReaderFromByteStream(byte_stream: Ptr, attributes: Ptr, reader: *mut Ptr) -> HRESULT;
}

#[link(name = "shlwapi", kind = "raw-dylib")]
extern "system" {
    fn SHCreateMemStream(data: *const u8, len: u32) -> Ptr;
}

#[link(name = "ole32", kind = "raw-dylib")]
extern "system" {
    fn CoInitializeEx(reserved: Ptr, coinit: u32) -> HRESULT;
}

#[link(name = "d3d11", kind = "raw-dylib")]
extern "system" {
    fn D3D11CreateDevice(
        adapter: Ptr,
        driver_type: u32,
        software: Ptr,
        flags: u32,
        levels: *const u32,
        level_count: u32,
        sdk_version: u32,
        device: *mut Ptr,
        level: *mut u32,
        context: *mut Ptr,
    ) -> HRESULT;
}

/// The function in vtable slot `slot` of COM object `obj`, cast to `F`.
unsafe fn slot<F: Copy>(obj: Ptr, slot: usize) -> F {
    let vtbl = *(obj as *const *const Ptr);
    std::mem::transmute_copy(&*vtbl.add(slot))
}

/// An owned COM pointer, released on drop.
struct Com(Ptr);

impl Com {
    fn new(p: Ptr) -> Option<Self> {
        (!p.is_null()).then_some(Self(p))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: slot 2 of every COM vtable is IUnknown::Release, and we own one reference.
        unsafe { slot::<unsafe extern "system" fn(Ptr) -> u32>(self.0, 2)(self.0) };
    }
}

fn check(hr: HRESULT, what: &str) -> Result<(), String> {
    if hr < 0 {
        Err(format!("{what} failed (0x{:08x})", hr as u32))
    } else {
        Ok(())
    }
}

// IMFAttributes and the interfaces that extend it (IMFMediaType, IMFSample).
unsafe fn set_u32(obj: Ptr, key: &GUID, v: u32) -> HRESULT {
    slot::<unsafe extern "system" fn(Ptr, *const GUID, u32) -> HRESULT>(obj, 21)(obj, key, v)
}
unsafe fn set_guid(obj: Ptr, key: &GUID, v: &GUID) -> HRESULT {
    slot::<unsafe extern "system" fn(Ptr, *const GUID, *const GUID) -> HRESULT>(obj, 24)(obj, key, v)
}
unsafe fn set_unknown(obj: Ptr, key: &GUID, v: Ptr) -> HRESULT {
    slot::<unsafe extern "system" fn(Ptr, *const GUID, Ptr) -> HRESULT>(obj, 27)(obj, key, v)
}
unsafe fn get_u32(obj: Ptr, key: &GUID) -> Option<u32> {
    let mut v = 0u32;
    let hr = slot::<unsafe extern "system" fn(Ptr, *const GUID, *mut u32) -> HRESULT>(obj, 7)(obj, key, &mut v);
    (hr >= 0).then_some(v)
}
unsafe fn get_u64(obj: Ptr, key: &GUID) -> Option<u64> {
    let mut v = 0u64;
    let hr = slot::<unsafe extern "system" fn(Ptr, *const GUID, *mut u64) -> HRESULT>(obj, 8)(obj, key, &mut v);
    (hr >= 0).then_some(v)
}

/// A Media Foundation source reader over an in-memory file, set to output RGB32.
pub struct Decoder {
    reader: Com,
    /// The Direct3D device and its manager stay alive as long as the reader uses them.
    _d3d: Option<(Com, Com)>,
    pub width: u32,
    pub height: u32,
    stride: i32,
    pub duration: f64,
    pub hardware: bool,
}

impl Decoder {
    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        // SAFETY: plain Win32 and COM calls with valid arguments. Every object we receive is
        // wrapped in `Com` at once, so it is released exactly once.
        unsafe {
            // The UI thread may already be in a single-threaded apartment (winit sets one up for
            // drag and drop). That is fine for Media Foundation, so a mode error is ignored.
            let _ = CoInitializeEx(null_mut(), 0);
            check(MFStartup(MF_VERSION, 0), "starting Media Foundation")?;
            match Self::open_with(bytes, true) {
                Ok(d) => Ok(d),
                Err(hw_err) => Self::open_with(bytes, false).map_err(|e| format!("{e} (with GPU decoding: {hw_err})")),
            }
        }
    }

    unsafe fn open_with(bytes: &[u8], hardware: bool) -> Result<Self, String> {
        let len = u32::try_from(bytes.len()).map_err(|_| "the video is larger than 4 GB".to_string())?;
        let stream = Com::new(SHCreateMemStream(bytes.as_ptr(), len)).ok_or("could not hold the video in memory")?;
        let mut p = null_mut();
        check(MFCreateMFByteStreamOnStream(stream.0, &mut p), "making a byte stream")?;
        let byte_stream = Com::new(p).ok_or("no byte stream")?;
        let mut p = null_mut();
        check(MFCreateAttributes(&mut p, 4), "making reader attributes")?;
        let attrs = Com::new(p).ok_or("no attributes")?;

        let mut d3d = None;
        if hardware {
            // A Direct3D 11 device with video support, shared with the decoder through a DXGI
            // device manager. That is what lets Media Foundation use the GPU's decoder.
            let (mut device, mut context, mut level) = (null_mut(), null_mut(), 0u32);
            check(
                D3D11CreateDevice(null_mut(), 1, null_mut(), 0x800 | 0x20, std::ptr::null(), 0, 7, &mut device, &mut level, &mut context),
                "making a Direct3D 11 video device",
            )?;
            let device = Com::new(device).ok_or("no Direct3D device")?;
            drop(Com::new(context));
            let mut mt = null_mut();
            let qi = slot::<unsafe extern "system" fn(Ptr, *const GUID, *mut Ptr) -> HRESULT>(device.0, 0);
            if qi(device.0, &IID_ID3D10_MULTITHREAD, &mut mt) >= 0 {
                let mt = Com::new(mt).ok_or("no multithread interface")?;
                // The decoder calls the device from its own threads.
                slot::<unsafe extern "system" fn(Ptr, i32) -> i32>(mt.0, 5)(mt.0, 1);
            }
            let (mut token, mut manager) = (0u32, null_mut());
            check(MFCreateDXGIDeviceManager(&mut token, &mut manager), "making a DXGI device manager")?;
            let manager = Com::new(manager).ok_or("no DXGI device manager")?;
            check(
                slot::<unsafe extern "system" fn(Ptr, Ptr, u32) -> HRESULT>(manager.0, 7)(manager.0, device.0, token),
                "handing the device to Media Foundation",
            )?;
            check(set_unknown(attrs.0, &MF_SOURCE_READER_D3D_MANAGER, manager.0), "setting the D3D manager")?;
            check(set_u32(attrs.0, &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1), "enabling hardware transforms")?;
            check(set_u32(attrs.0, &MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1), "enabling video processing")?;
            d3d = Some((device, manager));
        } else {
            check(set_u32(attrs.0, &MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1), "enabling video processing")?;
        }

        let mut p = null_mut();
        check(MFCreateSourceReaderFromByteStream(byte_stream.0, attrs.0, &mut p), "opening the video")?;
        let reader = Com::new(p).ok_or("no source reader")?;
        let select = slot::<unsafe extern "system" fn(Ptr, u32, i32) -> HRESULT>(reader.0, 4);
        select(reader.0, ALL_STREAMS, 0);
        check(select(reader.0, FIRST_VIDEO_STREAM, 1), "finding a video track")?;

        let mut p = null_mut();
        check(MFCreateMediaType(&mut p), "making a media type")?;
        let want = Com::new(p).ok_or("no media type")?;
        check(set_guid(want.0, &MF_MT_MAJOR_TYPE, &MF_MEDIA_TYPE_VIDEO), "setting the major type")?;
        check(set_guid(want.0, &MF_MT_SUBTYPE, &MF_VIDEO_FORMAT_RGB32), "setting RGB32")?;
        check(
            slot::<unsafe extern "system" fn(Ptr, u32, *mut u32, Ptr) -> HRESULT>(reader.0, 7)(reader.0, FIRST_VIDEO_STREAM, null_mut(), want.0),
            "asking for RGB frames (is the codec installed?)",
        )?;

        let mut p = null_mut();
        check(
            slot::<unsafe extern "system" fn(Ptr, u32, *mut Ptr) -> HRESULT>(reader.0, 6)(reader.0, FIRST_VIDEO_STREAM, &mut p),
            "reading the output format",
        )?;
        let current = Com::new(p).ok_or("no output format")?;
        let size = get_u64(current.0, &MF_MT_FRAME_SIZE).ok_or("the video has no frame size")?;
        let (width, height) = ((size >> 32) as u32, size as u32);
        if width == 0 || height == 0 {
            return Err("the video has no frame size".into());
        }
        let stride = get_u32(current.0, &MF_MT_DEFAULT_STRIDE).map_or(width as i32 * 4, |s| s as i32);

        let mut pv = PropVariant { vt: 0, reserved: [0; 3], value: 0, pad: 0 };
        let duration = if slot::<unsafe extern "system" fn(Ptr, u32, *const GUID, *mut PropVariant) -> HRESULT>(reader.0, 12)(
            reader.0,
            MEDIA_SOURCE,
            &MF_PD_DURATION,
            &mut pv,
        ) >= 0
        {
            pv.value as f64 / 1e7
        } else {
            0.0
        };
        Ok(Self { reader, _d3d: d3d, width, height, stride, duration, hardware })
    }

    /// The next picture and its time in seconds. `None` at the end of the video.
    pub fn next(&mut self) -> Result<Option<(f64, Frame)>, String> {
        // SAFETY: COM calls on objects this decoder owns, with out-pointers to locals. The locked
        // buffer is read only between Lock and Unlock, within the length the buffer reports.
        unsafe {
            loop {
                let (mut index, mut flags, mut time, mut sample) = (0u32, 0u32, 0i64, null_mut());
                check(
                    slot::<unsafe extern "system" fn(Ptr, u32, u32, *mut u32, *mut u32, *mut i64, *mut Ptr) -> HRESULT>(self.reader.0, 9)(
                        self.reader.0,
                        FIRST_VIDEO_STREAM,
                        0,
                        &mut index,
                        &mut flags,
                        &mut time,
                        &mut sample,
                    ),
                    "decoding",
                )?;
                let sample = Com::new(sample);
                if flags & END_OF_STREAM != 0 {
                    return Ok(None);
                }
                // A gap or a format change comes without a sample. Read on.
                let Some(sample) = sample else { continue };
                let mut buffer = null_mut();
                check(
                    slot::<unsafe extern "system" fn(Ptr, *mut Ptr) -> HRESULT>(sample.0, 41)(sample.0, &mut buffer),
                    "reading a frame",
                )?;
                let buffer = Com::new(buffer).ok_or("no frame buffer")?;
                let (mut data, mut max, mut len) = (null_mut::<u8>(), 0u32, 0u32);
                check(
                    slot::<unsafe extern "system" fn(Ptr, *mut *mut u8, *mut u32, *mut u32) -> HRESULT>(buffer.0, 3)(buffer.0, &mut data, &mut max, &mut len),
                    "locking a frame",
                )?;
                let bytes = std::slice::from_raw_parts(data, len as usize);
                let frame = self.to_rgba(bytes);
                slot::<unsafe extern "system" fn(Ptr) -> HRESULT>(buffer.0, 4)(buffer.0);
                return Ok(Some((time as f64 / 1e7, frame?)));
            }
        }
    }

    /// Converts a locked RGB32 buffer (B, G, R, X rows, `stride` apart; negative is bottom-up).
    fn to_rgba(&self, bytes: &[u8]) -> Result<Frame, String> {
        let (w, h) = (self.width as usize, self.height as usize);
        let pitch = self.stride.unsigned_abs() as usize;
        if pitch < w * 4 || bytes.len() < pitch * (h - 1) + w * 4 {
            return Err(format!("a frame came out smaller than {w}x{h}"));
        }
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = if self.stride < 0 { h - 1 - y } else { y };
            for px in bytes[row * pitch..row * pitch + w * 4].chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        Ok(Frame { width: self.width, height: self.height, rgba })
    }

    /// Goes back to the start.
    pub fn rewind(&mut self) -> Result<(), String> {
        let pv = PropVariant { vt: VT_I8, reserved: [0; 3], value: 0, pad: 0 };
        // SAFETY: SetCurrentPosition on the owned reader with a valid PROPVARIANT.
        unsafe {
            check(
                slot::<unsafe extern "system" fn(Ptr, *const GUID, *const PropVariant) -> HRESULT>(self.reader.0, 8)(self.reader.0, &GUID_NULL, &pv),
                "rewinding",
            )
        }
    }
}
