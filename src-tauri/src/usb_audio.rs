//! Raw USB isochronous audio streaming for Android.
//!
//! Uses Linux's usbdevice_fs ioctl interface to perform isochronous transfers
//! directly on a USB file descriptor obtained from Android's UsbDeviceConnection.
//! This bypasses Android's audio framework (Oboe/AAudio) which caps at ~192 kHz,
//! enabling sample rates up to 384 kHz (or 500 kHz if hardware supports it).
//!
//! Architecture modeled after batgizmo's nativeusb.cpp.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ── Shared types (all platforms) ────────────────────────────────────────

#[allow(dead_code)]
pub struct UsbStreamState {
    pub cancel_flag: Arc<AtomicBool>,
    pub is_recording: Arc<AtomicBool>,
    pub is_streaming: Arc<AtomicBool>,
    pub buffer: Arc<Mutex<UsbRecordingBuffer>>,
    pub sample_rate: u32,
    pub device_name: String,
}

pub struct UsbRecordingBuffer {
    samples_i16: Vec<i16>,
    pending_f32: Vec<f32>,
    pub total_samples: usize,
    pub sample_rate: u32,
}

#[allow(dead_code)]
impl UsbRecordingBuffer {
    fn new(sample_rate: u32) -> Self {
        Self {
            samples_i16: Vec::new(),
            pending_f32: Vec::new(),
            total_samples: 0,
            sample_rate,
        }
    }

    pub fn clear(&mut self) {
        self.samples_i16.clear();
        self.pending_f32.clear();
        self.total_samples = 0;
    }

    fn push_samples(&mut self, data: &[i16], recording: bool) {
        if recording {
            self.total_samples += data.len();
            self.samples_i16.extend_from_slice(data);
        }
        // Always push to pending for streaming/live display
        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
        self.pending_f32.extend_from_slice(&f32_data);
    }

    fn drain_pending(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.pending_f32)
    }
}

#[derive(Serialize)]
pub struct UsbStreamInfo {
    pub sample_rate: u32,
    pub device_name: String,
}

#[derive(Serialize)]
pub struct UsbStreamStatus {
    pub is_open: bool,
    pub is_streaming: bool,
    pub samples_recorded: usize,
    pub sample_rate: u32,
}

// ── Cross-platform functions ────────────────────────────────────────────

pub fn stop_usb_stream(state: &UsbStreamState) {
    state.cancel_flag.store(true, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn start_usb_emitter(
    app: tauri::AppHandle,
    buffer: Arc<Mutex<UsbRecordingBuffer>>,
    stop_flag: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        use tauri::Emitter;
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(80));
            let chunks = {
                let mut buf = buffer.lock().unwrap();
                buf.drain_pending()
            };
            if !chunks.is_empty() {
                let _ = app.emit("mic-audio-chunk", &chunks);
            }
        }
    });
}

pub fn get_usb_samples_f32(state: &UsbStreamState) -> Vec<f32> {
    let buf = state.buffer.lock().unwrap();
    buf.samples_i16
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect()
}

pub fn get_usb_status(state: &UsbStreamState) -> UsbStreamStatus {
    let buf = state.buffer.lock().unwrap();
    UsbStreamStatus {
        is_open: true,
        is_streaming: state.is_streaming.load(Ordering::Relaxed),
        samples_recorded: buf.total_samples,
        sample_rate: state.sample_rate,
    }
}

pub fn encode_usb_wav(state: &UsbStreamState) -> Result<Vec<u8>, String> {
    let buf = state.buffer.lock().unwrap();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: buf.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut writer =
        hound::WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV writer error: {}", e))?;

    for &s in &buf.samples_i16 {
        writer
            .write_sample(s)
            .map_err(|e| format!("WAV write error: {}", e))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {}", e))?;
    Ok(cursor.into_inner())
}

pub fn clear_usb_buffer(state: &UsbStreamState) {
    let mut buf = state.buffer.lock().unwrap();
    buf.clear();
}

// ── Android isochronous streaming ───────────────────────────────────────

#[cfg(target_os = "android")]
pub fn start_usb_stream(
    fd: i32,
    endpoint_address: u32,
    max_packet_size: u32,
    sample_rate: u32,
    num_channels: u32,
    device_name: String,
    app: tauri::AppHandle,
) -> Result<UsbStreamState, String> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let is_streaming = Arc::new(AtomicBool::new(true));
    let is_recording = Arc::new(AtomicBool::new(false));
    let buffer = Arc::new(Mutex::new(UsbRecordingBuffer::new(sample_rate)));

    let cancel = cancel_flag.clone();
    let streaming = is_streaming.clone();
    let recording = is_recording.clone();
    let buf = buffer.clone();
    let channels = num_channels as usize;

    std::thread::spawn(move || {
        let result = isochronous::run_isochronous_loop(
            fd,
            endpoint_address,
            max_packet_size,
            channels,
            &cancel,
            &recording,
            &buf,
        );

        streaming.store(false, Ordering::Relaxed);

        match result {
            Ok(()) => eprintln!("USB stream ended normally"),
            Err(e) => eprintln!("USB stream error: {}", e),
        }
    });

    // Start emitter for streaming audio chunks to the frontend
    start_usb_emitter(app, buffer.clone(), cancel_flag.clone());

    Ok(UsbStreamState {
        cancel_flag,
        is_recording,
        buffer,
        sample_rate,
        is_streaming,
        device_name,
    })
}

#[cfg(not(target_os = "android"))]
pub fn start_usb_stream(
    _fd: i32,
    _endpoint_address: u32,
    _max_packet_size: u32,
    _sample_rate: u32,
    _num_channels: u32,
    _device_name: String,
    _app: tauri::AppHandle,
) -> Result<UsbStreamState, String> {
    Err("USB audio streaming is only supported on Android".into())
}

// ── Linux USB isochronous implementation (Android only) ─────────────────

#[cfg(target_os = "android")]
mod isochronous {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use super::UsbRecordingBuffer;

    // ioctl direction bits
    const _IOC_WRITE: u32 = 1;
    const _IOC_READ: u32 = 2;
    const _IOC_NRBITS: u32 = 8;
    const _IOC_TYPEBITS: u32 = 8;
    const _IOC_SIZEBITS: u32 = 14;
    const _IOC_NRSHIFT: u32 = 0;
    const _IOC_TYPESHIFT: u32 = _IOC_NRSHIFT + _IOC_NRBITS;
    const _IOC_SIZESHIFT: u32 = _IOC_TYPESHIFT + _IOC_TYPEBITS;
    const _IOC_DIRSHIFT: u32 = _IOC_SIZESHIFT + _IOC_SIZEBITS;

    const fn _ioc(dir: u32, ty: u32, nr: u32, size: u32) -> u32 {
        (dir << _IOC_DIRSHIFT)
            | (ty << _IOC_TYPESHIFT)
            | (nr << _IOC_NRSHIFT)
            | (size << _IOC_SIZESHIFT)
    }

    const fn _ior(ty: u32, nr: u32, size: u32) -> u32 {
        _ioc(_IOC_READ, ty, nr, size)
    }

    const fn _iow(ty: u32, nr: u32, size: u32) -> u32 {
        _ioc(_IOC_WRITE, ty, nr, size)
    }

    const USBDEVFS_MAGIC: u32 = b'U' as u32;

    const USBDEVFS_SUBMITURB: u32 =
        _ior(USBDEVFS_MAGIC, 10, std::mem::size_of::<UsbdevfsUrb>() as u32);
    const USBDEVFS_DISCARDURB: u32 = _ioc(0, USBDEVFS_MAGIC, 11, 0);
    const USBDEVFS_REAPURB: u32 =
        _iow(USBDEVFS_MAGIC, 12, std::mem::size_of::<*mut std::ffi::c_void>() as u32);
    const USBDEVFS_REAPURBNDELAY: u32 =
        _iow(USBDEVFS_MAGIC, 13, std::mem::size_of::<*mut std::ffi::c_void>() as u32);

    const USBDEVFS_URB_TYPE_ISO: u8 = 0;
    const USBDEVFS_URB_ISO_ASAP: u32 = 0x02;

    #[repr(C)]
    #[derive(Clone)]
    struct UsbdevfsIsoPacketDesc {
        length: u32,
        actual_length: u32,
        status: u32,
    }

    #[repr(C)]
    struct UsbdevfsUrb {
        urb_type: u8,
        endpoint: u8,
        status: i32,
        flags: u32,
        buffer: *mut u8,
        buffer_length: i32,
        actual_length: i32,
        start_frame: i32,
        number_of_packets: i32,
        error_count: i32,
        signr: u32,
        usercontext: *mut std::ffi::c_void,
    }

    const MAX_CHANNELS: usize = 2;
    const MAX_SAMPLES_PER_FRAME: usize = 501; // 500 kHz = 500 samples/ms + slack
    const URBS_TO_JUGGLE: usize = 10;
    const PACKETS_PER_URB: usize = 25; // 25ms worth of USB frames
    const MAX_DATA_POINTS_PER_URB: usize = MAX_SAMPLES_PER_FRAME * MAX_CHANNELS * PACKETS_PER_URB;

    #[repr(C)]
    struct UrbWithPackets {
        urb: UsbdevfsUrb,
        packet_desc: [UsbdevfsIsoPacketDesc; PACKETS_PER_URB],
    }

    pub fn run_isochronous_loop(
        fd: i32,
        endpoint_address: u32,
        max_packet_size: u32,
        num_channels: usize,
        cancel: &AtomicBool,
        is_recording: &AtomicBool,
        buffer: &Arc<Mutex<UsbRecordingBuffer>>,
    ) -> Result<(), String> {
        let requested_bytes_per_frame = max_packet_size as usize;

        let mut audio_buffers: Vec<Vec<u8>> = (0..URBS_TO_JUGGLE)
            .map(|_| vec![0u8; MAX_DATA_POINTS_PER_URB * 2])
            .collect();

        let mut urbs: Vec<Box<UrbWithPackets>> = (0..URBS_TO_JUGGLE)
            .map(|i| {
                let mut urb_box = Box::new(UrbWithPackets {
                    urb: UsbdevfsUrb {
                        urb_type: USBDEVFS_URB_TYPE_ISO,
                        endpoint: (endpoint_address | 0x80) as u8,
                        status: 0,
                        flags: USBDEVFS_URB_ISO_ASAP,
                        buffer: audio_buffers[i].as_mut_ptr(),
                        buffer_length: (MAX_DATA_POINTS_PER_URB * 2) as i32,
                        actual_length: 0,
                        start_frame: 0,
                        number_of_packets: PACKETS_PER_URB as i32,
                        error_count: 0,
                        signr: 0,
                        usercontext: std::ptr::null_mut(),
                    },
                    packet_desc: std::array::from_fn(|_| UsbdevfsIsoPacketDesc {
                        length: requested_bytes_per_frame as u32,
                        actual_length: 0,
                        status: 0,
                    }),
                });
                urb_box.urb.usercontext =
                    &*urb_box as *const UrbWithPackets as *mut std::ffi::c_void;
                urb_box
            })
            .collect();

        for (i, urb) in urbs.iter_mut().enumerate() {
            urb.urb.buffer = audio_buffers[i].as_mut_ptr();
        }

        // Submit all URBs
        for urb in &urbs {
            let ret = unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_SUBMITURB as libc::c_int,
                    &urb.urb as *const UsbdevfsUrb,
                )
            };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("USBDEVFS_SUBMITURB failed: {}", err));
            }
        }

        let mut balls_in_air = URBS_TO_JUGGLE;
        let mut mono_buf: Vec<i16> = Vec::with_capacity(MAX_DATA_POINTS_PER_URB);

        // Main juggling loop
        while !cancel.load(Ordering::Relaxed) || balls_in_air > 0 {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN | libc::POLLOUT,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if poll_ret <= 0 {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                discard_urbs(fd, &urbs);
                return Err(
                    "USB stream poll timeout — device may have disconnected".to_string(),
                );
            }

            let mut urb_reaped: *mut UsbdevfsUrb = std::ptr::null_mut();
            let ret = unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_REAPURB as libc::c_int,
                    &mut urb_reaped as *mut *mut UsbdevfsUrb,
                )
            };

            if ret != 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                if err.raw_os_error() == Some(libc::ENODEV) {
                    eprintln!("USB device disconnected");
                    break;
                }
                eprintln!("USBDEVFS_REAPURB error: {}", err);
                continue;
            }

            balls_in_air -= 1;

            // Process the reaped URB's audio data
            if !cancel.load(Ordering::Relaxed) {
                let reaped = unsafe { &*urb_reaped };
                let urb_with_packets =
                    unsafe { &*(reaped.usercontext as *const UrbWithPackets) };

                mono_buf.clear();

                let mut src_byte_offset: usize = 0;
                for packet in &urb_with_packets.packet_desc {
                    let actual_bytes = packet.actual_length as usize;
                    let actual_samples = actual_bytes / 2;

                    if actual_samples > 0 {
                        let src = unsafe {
                            std::slice::from_raw_parts(
                                (reaped.buffer as *const u8).add(src_byte_offset)
                                    as *const i16,
                                actual_samples,
                            )
                        };

                        if num_channels == 2 {
                            for chunk in src.chunks(2) {
                                if chunk.len() == 2 {
                                    let avg =
                                        ((chunk[0] as i32 + chunk[1] as i32) >> 1) as i16;
                                    mono_buf.push(avg);
                                }
                            }
                        } else {
                            mono_buf.extend_from_slice(src);
                        }
                    }

                    src_byte_offset += packet.length as usize;
                }

                if !mono_buf.is_empty() {
                    let recording = is_recording.load(Ordering::Relaxed);
                    if let Ok(mut buf) = buffer.lock() {
                        buf.push_samples(&mono_buf, recording);
                    }
                }
            }

            // Resubmit unless cancelling
            if !cancel.load(Ordering::Relaxed) {
                let ret = unsafe {
                    libc::ioctl(fd, USBDEVFS_SUBMITURB as libc::c_int, urb_reaped)
                };
                if ret == 0 {
                    balls_in_air += 1;
                } else {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::ENODEV) {
                        break;
                    }
                    eprintln!("USBDEVFS_SUBMITURB resubmit error: {}", err);
                }
            }
        }

        Ok(())
    }

    fn discard_urbs(fd: i32, urbs: &[Box<UrbWithPackets>]) {
        for urb in urbs {
            unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_DISCARDURB as libc::c_int,
                    &urb.urb as *const UsbdevfsUrb,
                );
            }
        }
        for _ in urbs {
            let mut urb_reaped: *mut UsbdevfsUrb = std::ptr::null_mut();
            unsafe {
                libc::ioctl(
                    fd,
                    USBDEVFS_REAPURBNDELAY as libc::c_int,
                    &mut urb_reaped as *mut *mut UsbdevfsUrb,
                );
            }
        }
    }
}
