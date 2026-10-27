//! The Windows backend: WASAPI on the default devices (the "console" role),
//! each stream on its own thread, woken by the audio engine.
//!
//! - **Capture** asks for **exclusive mode**: Drywire then talks to the
//!   driver directly, with the device's smallest period and no mixing. If
//!   another app holds the microphone, the device refuses exclusive access,
//!   or no 48 kHz format fits, it falls back to shared mode, and the mode
//!   it got is reported.
//! - **Playback** stays in **shared mode**, because the game plays on the
//!   same headset, with the smallest period `IAudioClient3` offers, which
//!   the device reports and `drywire-cli devices` shows.
//!
//! Both streams ask for the raw signal path (`AUDCLNT_STREAMOPTIONS_RAW`),
//! which bypasses the driver's effects where the device allows it, and the
//! `GameChat` category, which unlike `Communications` never makes Windows
//! lower the volume of other apps. No voice processing is ever enabled.
//!
//! The bindings are Microsoft's `windows` crate (ADR 0013). This file only
//! compiles on Windows; `cargo check --target x86_64-pc-windows-msvc`
//! checks it from another system.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_SHAREMODE_EXCLUSIVE,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDCLNT_STREAMOPTIONS_RAW, AudioCategory_GameChat, AudioClientProperties, EDataFlow,
    IAudioCaptureClient, IAudioClient, IAudioClient2, IAudioClient3, IAudioRenderClient, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVEFORMATEXTENSIBLE_0, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};
use windows::core::{GUID, Interface, w};

use super::{
    AudioError, AudioIo, AudioOptions, Direction, InputCallback, OutputCallback, SAMPLE_RATE,
    StreamInfo,
};

/// Format tags of `WAVEFORMATEX` (mmreg.h).
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
/// `KSDATAFORMAT_SUBTYPE_PCM` and `_IEEE_FLOAT` (ksmedia.h): the format tag
/// inside the GUID template `0000xxxx-0000-0010-8000-00aa00389b71`.
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

/// WASAPI counts time in units of 100 ns (`REFERENCE_TIME`).
const UNITS_PER_SECOND: i64 = 10_000_000;

fn error(what: &str, e: windows::core::Error) -> AudioError {
    AudioError(format!("WASAPI: {what}: {e}"))
}

/// The sample layout of a stream, read from its `WAVEFORMATEX`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Layout {
    channels: usize,
    rate: u32,
    /// Bytes per sample, and whether samples are floats.
    width: usize,
    float: bool,
}

impl Layout {
    /// Reads a format WASAPI gave us.
    ///
    /// # Safety
    ///
    /// `format` points to a valid `WAVEFORMATEX`, followed by the rest of a
    /// `WAVEFORMATEXTENSIBLE` when its tag says so.
    unsafe fn read(format: *const WAVEFORMATEX) -> Result<Layout, AudioError> {
        // SAFETY: the caller's promise; the struct is packed, so it is read
        // unaligned.
        let f = unsafe { format.read_unaligned() };
        let mut tag = f.wFormatTag;
        if tag == WAVE_FORMAT_EXTENSIBLE {
            // SAFETY: the tag says the extensible part follows.
            let x = unsafe { format.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
            let sub = x.SubFormat;
            tag = if sub == SUBTYPE_IEEE_FLOAT {
                WAVE_FORMAT_IEEE_FLOAT
            } else if sub == SUBTYPE_PCM {
                WAVE_FORMAT_PCM
            } else {
                0
            };
        }
        let layout = Layout {
            channels: usize::from(f.nChannels),
            rate: f.nSamplesPerSec,
            width: usize::from(f.wBitsPerSample / 8),
            float: tag == WAVE_FORMAT_IEEE_FLOAT,
        };
        let known = match tag {
            WAVE_FORMAT_IEEE_FLOAT => layout.width == 4,
            WAVE_FORMAT_PCM => (2..=4).contains(&layout.width),
            _ => false,
        };
        if !known || layout.channels == 0 {
            let bits = f.wBitsPerSample;
            return Err(AudioError(format!(
                "WASAPI: unsupported sample format (tag {tag:#x}, {bits} bits, {} channels)",
                layout.channels
            )));
        }
        Ok(layout)
    }

    /// An extensible format with this layout, for asking WASAPI.
    fn to_format(self) -> WAVEFORMATEXTENSIBLE {
        let block = (self.channels * self.width) as u16;
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_EXTENSIBLE,
                nChannels: self.channels as u16,
                nSamplesPerSec: self.rate,
                nAvgBytesPerSec: self.rate * u32::from(block),
                nBlockAlign: block,
                wBitsPerSample: (self.width * 8) as u16,
                cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: (self.width * 8) as u16,
            },
            // Front left, front right, … in order: the default layout.
            dwChannelMask: u32::MAX >> (32 - self.channels.min(32)),
            SubFormat: if self.float {
                SUBTYPE_IEEE_FLOAT
            } else {
                SUBTYPE_PCM
            },
        }
    }

    /// One sample, scaled to `[-1, 1]`.
    fn read_sample(self, b: &[u8]) -> f32 {
        match (self.float, self.width) {
            (true, _) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (false, 2) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
            (false, 3) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2_147_483_648.0,
            (false, _) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        }
    }

    fn write_sample(self, b: &mut [u8], x: f32) {
        let x = x.clamp(-1.0, 1.0);
        match (self.float, self.width) {
            (true, _) => b.copy_from_slice(&x.to_le_bytes()),
            (false, 2) => b.copy_from_slice(&((x * 32_767.0) as i16).to_le_bytes()),
            (false, 3) => b.copy_from_slice(&((x * 8_388_607.0) as i32).to_le_bytes()[..3]),
            (false, _) => b.copy_from_slice(&((x * 2_147_483_647.0) as i32).to_le_bytes()),
        }
    }
}

/// Initializes COM for the calling thread, and uninitializes it when
/// dropped. WASAPI objects then belong to that thread.
struct Com;

impl Com {
    fn init() -> Result<Com, AudioError> {
        // SAFETY: called once per thread, paired with CoUninitialize in Drop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|e| error("initializing COM", e))?;
        Ok(Com)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful CoInitializeEx of init.
        unsafe { CoUninitialize() }
    }
}

/// A Win32 event, closed when dropped.
struct Event(HANDLE);

impl Event {
    fn new() -> Result<Event, AudioError> {
        // SAFETY: an unnamed auto-reset event with default security.
        let handle = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| error("creating an event", e))?;
        Ok(Event(handle))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn default_device(direction: Direction) -> Result<IMMDevice, AudioError> {
    // SAFETY: plain COM calls on this thread, which initialized COM.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| error("creating the device enumerator", e))?;
        let flow: EDataFlow = match direction {
            Direction::Input => eCapture,
            Direction::Output => eRender,
        };
        enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .map_err(|e| error("finding the default device", e))
    }
}

fn device_name(device: &IMMDevice) -> String {
    // SAFETY: plain COM calls; the property store is read-only.
    unsafe {
        device
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "default device".into())
    }
}

/// The device's mix format, exactly as WASAPI returns it: shared mode
/// accepts that format only, channel mask and valid bits included, so it is
/// passed back as is rather than rebuilt. Freed when dropped.
struct MixFormat {
    format: *mut WAVEFORMATEX,
    layout: Layout,
}

impl MixFormat {
    fn of(client: &IAudioClient) -> Result<MixFormat, AudioError> {
        // SAFETY: GetMixFormat returns a valid format allocated with
        // CoTaskMemAlloc, which Drop frees.
        unsafe {
            let format = client
                .GetMixFormat()
                .map_err(|e| error("reading the mix format", e))?;
            // Owned from here on, so an unreadable format is freed too.
            let mut mix = MixFormat {
                format,
                layout: Layout::default(),
            };
            mix.layout = Layout::read(mix.format)?;
            Ok(mix)
        }
    }
}

impl Drop for MixFormat {
    fn drop(&mut self) {
        // SAFETY: allocated by GetMixFormat, freed once.
        unsafe { CoTaskMemFree(Some(self.format as *const c_void)) }
    }
}

/// Asks for the raw signal path and a category that never ducks other
/// apps. Devices that do not support raw mode refuse it; that only means
/// their effects stay on, so the refusal is not an error.
fn set_properties(client: &IAudioClient) {
    let properties = AudioClientProperties {
        cbSize: size_of::<AudioClientProperties>() as u32,
        bIsOffload: false.into(),
        eCategory: AudioCategory_GameChat,
        Options: AUDCLNT_STREAMOPTIONS_RAW,
    };
    if let Ok(client2) = client.cast::<IAudioClient2>() {
        // SAFETY: a valid properties structure, set before Initialize.
        let _ = unsafe { client2.SetClientProperties(&properties) };
    }
}

fn frames_of(duration: i64, rate: u32) -> u32 {
    (duration * i64::from(rate) / UNITS_PER_SECOND) as u32
}

fn duration_of(frames: u32, rate: u32) -> i64 {
    // Rounded to the nearest unit, as Microsoft's own samples do.
    (UNITS_PER_SECOND * i64::from(frames) + i64::from(rate) / 2) / i64::from(rate)
}

/// An initialized stream, ready to start.
struct Stream {
    client: IAudioClient,
    layout: Layout,
    info: StreamInfo,
}

/// Shared mode with the smallest `IAudioClient3` period, when the mix
/// format runs at 48 kHz. At any other rate, the audio engine resamples
/// our 48 kHz stream (`AUTOCONVERTPCM`), which only the classic
/// `Initialize` allows, with its default period.
fn open_shared(
    device: &IMMDevice,
    direction: Direction,
    options: AudioOptions,
    note: &str,
) -> Result<Stream, AudioError> {
    // SAFETY: plain COM calls on this thread; every pointer passed points
    // to a live local.
    unsafe {
        let client: IAudioClient3 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| error("activating the audio client", e))?;
        let base: IAudioClient = client.cast().map_err(|e| error("IAudioClient", e))?;
        set_properties(&base);
        let mix_format = MixFormat::of(&base)?;
        let mix = mix_format.layout;
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        let (layout, min_period, mode) = if mix.rate == SAMPLE_RATE {
            let format = mix_format.format as *const WAVEFORMATEX;
            let (mut default, mut fundamental, mut min, mut max) = (0, 0, 0, 0);
            client
                .GetSharedModeEnginePeriod(
                    format,
                    &mut default,
                    &mut fundamental,
                    &mut min,
                    &mut max,
                )
                .map_err(|e| error("reading the shared periods", e))?;
            // A requested period is rounded to the engine's step.
            let period = match options.buffer_frames {
                Some(frames) => {
                    (frames.clamp(min, max) / fundamental.max(1)).max(1) * fundamental.max(1)
                }
                None => min,
            };
            client
                .InitializeSharedAudioStream(flags, period, format, None)
                .map_err(|e| error("initializing the shared stream", e))?;
            (
                mix,
                min,
                format!("WASAPI shared, IAudioClient3 period {period} frames{note}"),
            )
        } else {
            let ours = Layout {
                rate: SAMPLE_RATE,
                width: 4,
                float: true,
                ..mix
            };
            let format = ours.to_format();
            let flags = flags
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            base.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                0,
                0,
                &format as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
                None,
            )
            .map_err(|e| error("initializing the shared stream", e))?;
            let mode = format!(
                "WASAPI shared, resampled from {} Hz by Windows, default period{note}",
                mix.rate
            );
            (ours, 0, mode)
        };
        let info = describe(device, &base, direction, layout, min_period, mode)?;
        Ok(Stream {
            client: base,
            layout,
            info,
        })
    }
}

/// Exclusive mode at 48 kHz, in the first format the device accepts, with
/// its smallest period (or the one asked for).
fn open_exclusive(device: &IMMDevice, options: AudioOptions) -> Result<Stream, AudioError> {
    // Float first, then integers from the widest; mono before stereo.
    let candidates = [
        (1, 4, true),
        (2, 4, true),
        (1, 4, false),
        (2, 4, false),
        (1, 3, false),
        (2, 3, false),
        (1, 2, false),
        (2, 2, false),
    ];
    // SAFETY: plain COM calls on this thread; every pointer passed points
    // to a live local.
    unsafe {
        let probe: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| error("activating the audio client", e))?;
        let layout = candidates
            .iter()
            .map(|&(channels, width, float)| Layout {
                channels,
                rate: SAMPLE_RATE,
                width,
                float,
            })
            .find(|layout| {
                let format = layout.to_format();
                probe
                    .IsFormatSupported(
                        AUDCLNT_SHAREMODE_EXCLUSIVE,
                        &format as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX,
                        None,
                    )
                    .is_ok()
            })
            .ok_or_else(|| AudioError("no 48 kHz format in exclusive mode".into()))?;
        let (mut default_period, mut min_period) = (0i64, 0i64);
        probe
            .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))
            .map_err(|e| error("reading the device period", e))?;
        drop(probe);

        let min_frames = frames_of(min_period, SAMPLE_RATE);
        let mut period = match options.buffer_frames {
            Some(frames) => duration_of(frames.max(min_frames), SAMPLE_RATE),
            None => min_period,
        };
        let format = layout.to_format();
        let format = &format as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        // In event mode, the buffer and the period are the same duration.
        let mut client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| error("activating the audio client", e))?;
        set_properties(&client);
        if let Err(e) = client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            flags,
            period,
            period,
            format,
            None,
        ) {
            if e.code() != AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
                return Err(error("initializing the exclusive stream", e));
            }
            // The driver wants a whole number of its own blocks: ask again
            // with the size it proposes, on a fresh client.
            let frames = client
                .GetBufferSize()
                .map_err(|e| error("reading the aligned buffer", e))?;
            period = duration_of(frames, SAMPLE_RATE);
            client = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| error("activating the audio client", e))?;
            set_properties(&client);
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                    flags,
                    period,
                    period,
                    format,
                    None,
                )
                .map_err(|e| error("initializing the exclusive stream", e))?;
        }
        let mode = "WASAPI exclusive".to_string();
        let info = describe(device, &client, Direction::Input, layout, min_frames, mode)?;
        Ok(Stream {
            client,
            layout,
            info,
        })
    }
}

fn describe(
    device: &IMMDevice,
    client: &IAudioClient,
    direction: Direction,
    layout: Layout,
    min_buffer_frames: u32,
    mode: String,
) -> Result<StreamInfo, AudioError> {
    // SAFETY: plain COM calls on an initialized client.
    let (buffer, latency) = unsafe {
        (
            client
                .GetBufferSize()
                .map_err(|e| error("reading the buffer size", e))?,
            client.GetStreamLatency().unwrap_or(0),
        )
    };
    let latency = frames_of(latency, layout.rate);
    Ok(StreamInfo {
        direction,
        device: device_name(device),
        mode,
        device_rate: layout.rate,
        buffer_frames: buffer,
        min_buffer_frames: min_buffer_frames.max(1),
        latency_frames: latency,
        latency_parts: vec![("stream", latency)],
        // WASAPI does not say; the endpoint's form factor would, later.
        bluetooth: false,
    })
}

/// Raises the calling thread to the "Pro Audio" scheduling class, and
/// lowers it back when dropped.
struct ProAudio(Option<HANDLE>);

impl ProAudio {
    fn enter() -> ProAudio {
        let mut task = 0u32;
        // SAFETY: a valid task name and a valid place for the task index.
        ProAudio(unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) }.ok())
    }
}

impl Drop for ProAudio {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            // SAFETY: the handle came from AvSetMmThreadCharacteristicsW.
            let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
    }
}

/// Opens the capture stream, reports it, and runs it until `stop`.
fn run_capture(
    options: AudioOptions,
    mut callback: InputCallback,
    stop: Arc<AtomicBool>,
    report: Sender<Result<StreamInfo, AudioError>>,
) {
    // COM outlives every WASAPI object of this thread, created below.
    let _com = match Com::init() {
        Ok(com) => com,
        Err(e) => {
            let _ = report.send(Err(e));
            return;
        }
    };
    let result = default_device(Direction::Input)
        .and_then(|device| match open_exclusive(&device, options) {
            Ok(stream) => Ok(stream),
            Err(refused) => open_shared(
                &device,
                Direction::Input,
                options,
                &format!(" (exclusive refused: {refused})"),
            ),
        })
        .and_then(|stream| capture_loop(&stream, &mut callback, &stop, &report));
    finish("capture", result, &report);
}

/// Sends an error that happened before the stream started to `start`. One
/// that happens later has nobody waiting for it any more: the stream
/// stops, and the error is printed rather than lost.
fn finish(
    name: &str,
    result: Result<(), AudioError>,
    report: &Sender<Result<StreamInfo, AudioError>>,
) {
    if let Err(e) = result
        && let Err(unsent) = report.send(Err(e))
        && let Err(e) = unsent.0
    {
        eprintln!("drywire: audio {name} stopped: {e}");
    }
}

fn capture_loop(
    stream: &Stream,
    callback: &mut InputCallback,
    stop: &AtomicBool,
    report: &Sender<Result<StreamInfo, AudioError>>,
) -> Result<(), AudioError> {
    let event = Event::new()?;
    let layout = stream.layout;
    let frame_bytes = layout.channels * layout.width;
    let mut mono = vec![0.0f32; 4096];
    // SAFETY: plain COM calls on this thread's initialized client; the
    // buffer WASAPI lends is read for exactly the frames it says, then
    // released.
    unsafe {
        stream
            .client
            .SetEventHandle(event.0)
            .map_err(|e| error("setting the event", e))?;
        let capture: IAudioCaptureClient = stream
            .client
            .GetService()
            .map_err(|e| error("getting the capture client", e))?;
        stream
            .client
            .Start()
            .map_err(|e| error("starting capture", e))?;
        let _ = report.send(Ok(stream.info.clone()));
        let _priority = ProAudio::enter();
        while !stop.load(Ordering::Relaxed) {
            if WaitForSingleObject(event.0, 100) != WAIT_OBJECT_0 {
                continue;
            }
            while capture
                .GetNextPacketSize()
                .map_err(|e| error("capture", e))?
                > 0
            {
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(|e| error("reading captured audio", e))?;
                let n = frames as usize;
                if mono.len() < n {
                    mono.resize(n, 0.0);
                }
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    mono[..n].fill(0.0);
                } else {
                    let bytes = std::slice::from_raw_parts(data, n * frame_bytes);
                    // The first channel: a microphone's own channel, as the
                    // Core Audio backend takes it.
                    for (out, frame) in mono[..n].iter_mut().zip(bytes.chunks_exact(frame_bytes)) {
                        *out = layout.read_sample(&frame[..layout.width]);
                    }
                }
                capture
                    .ReleaseBuffer(frames)
                    .map_err(|e| error("releasing captured audio", e))?;
                callback(&mono[..n]);
            }
        }
        let _ = stream.client.Stop();
    }
    Ok(())
}

/// Opens the playback stream, reports it, and runs it until `stop`.
fn run_render(
    options: AudioOptions,
    mut callback: OutputCallback,
    stop: Arc<AtomicBool>,
    report: Sender<Result<StreamInfo, AudioError>>,
) {
    let _com = match Com::init() {
        Ok(com) => com,
        Err(e) => {
            let _ = report.send(Err(e));
            return;
        }
    };
    let result = default_device(Direction::Output)
        .and_then(|device| open_shared(&device, Direction::Output, options, ""))
        .and_then(|stream| render_loop(&stream, &mut callback, &stop, &report));
    finish("playback", result, &report);
}

fn render_loop(
    stream: &Stream,
    callback: &mut OutputCallback,
    stop: &AtomicBool,
    report: &Sender<Result<StreamInfo, AudioError>>,
) -> Result<(), AudioError> {
    let event = Event::new()?;
    let layout = stream.layout;
    let frame_bytes = layout.channels * layout.width;
    // SAFETY: plain COM calls on this thread's initialized client; the
    // buffer WASAPI lends is written for exactly the frames asked, then
    // released.
    unsafe {
        stream
            .client
            .SetEventHandle(event.0)
            .map_err(|e| error("setting the event", e))?;
        let render: IAudioRenderClient = stream
            .client
            .GetService()
            .map_err(|e| error("getting the render client", e))?;
        let buffer = stream
            .client
            .GetBufferSize()
            .map_err(|e| error("reading the buffer size", e))?;
        let mut mono = vec![0.0f32; buffer as usize];
        stream
            .client
            .Start()
            .map_err(|e| error("starting playback", e))?;
        let _ = report.send(Ok(stream.info.clone()));
        let _priority = ProAudio::enter();
        while !stop.load(Ordering::Relaxed) {
            if WaitForSingleObject(event.0, 100) != WAIT_OBJECT_0 {
                continue;
            }
            let padding = stream
                .client
                .GetCurrentPadding()
                .map_err(|e| error("playback", e))?;
            let free = buffer.saturating_sub(padding);
            if free == 0 {
                continue;
            }
            let n = free as usize;
            callback(&mut mono[..n]);
            let data = render
                .GetBuffer(free)
                .map_err(|e| error("getting the playback buffer", e))?;
            let bytes = std::slice::from_raw_parts_mut(data, n * frame_bytes);
            for (frame, sample) in bytes.chunks_exact_mut(frame_bytes).zip(&mono) {
                for channel in frame.chunks_exact_mut(layout.width) {
                    layout.write_sample(channel, *sample);
                }
            }
            render
                .ReleaseBuffer(free, 0)
                .map_err(|e| error("releasing the playback buffer", e))?;
        }
        let _ = stream.client.Stop();
    }
    Ok(())
}

/// Reports what the default devices would give, without starting a
/// stream: it initializes each stream as `start` would, reads what WASAPI
/// says, and releases it.
pub fn probe(options: AudioOptions) -> Vec<Result<StreamInfo, AudioError>> {
    let _com = match Com::init() {
        Ok(com) => com,
        Err(e) => return vec![Err(e)],
    };
    let input = default_device(Direction::Input).and_then(|device| {
        open_exclusive(&device, options)
            .or_else(|refused| {
                open_shared(
                    &device,
                    Direction::Input,
                    options,
                    &format!(" (exclusive refused: {refused})"),
                )
            })
            .map(|stream| stream.info)
    });
    let output = default_device(Direction::Output)
        .and_then(|device| open_shared(&device, Direction::Output, options, ""))
        .map(|stream| stream.info);
    vec![input, output]
}

/// The default devices of this PC. See the module documentation.
pub struct Wasapi {
    options: AudioOptions,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Wasapi {
    /// The backend, not started yet.
    pub fn new(options: AudioOptions) -> Wasapi {
        Wasapi {
            options,
            stop: Arc::new(AtomicBool::new(false)),
            threads: Vec::new(),
        }
    }
}

impl AudioIo for Wasapi {
    fn start(
        &mut self,
        input: InputCallback,
        output: OutputCallback,
    ) -> Result<Vec<StreamInfo>, AudioError> {
        if !self.threads.is_empty() {
            return Err(AudioError("already started".into()));
        }
        self.stop.store(false, Ordering::Relaxed);
        let (report, reports) = channel();
        let spawn = |name: &str, run: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .name(name.into())
                .spawn(run)
                .map_err(|e| AudioError(format!("starting the {name} thread: {e}")))
        };
        let (options, stop, r) = (self.options, Arc::clone(&self.stop), report.clone());
        self.threads.push(spawn(
            "drywire-capture",
            Box::new(move || run_capture(options, input, stop, r)),
        )?);
        let (options, stop) = (self.options, Arc::clone(&self.stop));
        self.threads.push(spawn(
            "drywire-render",
            Box::new(move || run_render(options, output, stop, report)),
        )?);

        // Each thread reports once it runs, or why it could not start.
        let mut infos = Vec::new();
        for _ in 0..2 {
            match reports.recv() {
                Ok(Ok(info)) => infos.push(info),
                Ok(Err(e)) => {
                    self.stop();
                    return Err(e);
                }
                Err(_) => {
                    self.stop();
                    return Err(AudioError("an audio thread stopped before starting".into()));
                }
            }
        }
        infos.sort_by_key(|i| i.direction == Direction::Output);
        Ok(infos)
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for Wasapi {
    fn drop(&mut self) {
        self.stop();
    }
}
