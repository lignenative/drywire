//! The macOS backend: the default input and output devices, each through its
//! own HAL output unit (AUHAL), with the smallest I/O buffer the device
//! accepts.
//!
//! The HAL output unit is the lowest layer of Core Audio that still converts
//! formats for us. Unlike the voice processing unit it applies no echo
//! cancellation, gain control or noise suppression: what the microphone
//! hears is what we get. We ask it for mono 48 kHz float on our side. On
//! output, when the device runs at another rate, the unit resamples. On
//! input it cannot: the HAL output unit never converts the sample rate of
//! what it captures, so a microphone must run at 48 kHz, and `start` says
//! how to set it when it does not.
//!
//! The bindings below are written by hand (ADR 0013): Core Audio is a plain
//! C API, and the few functions, structures and constants we need are
//! declared exactly as in Apple's headers (`AudioHardware.h`,
//! `AudioUnitProperties.h`, `AUComponent.h`, `AudioComponent.h`), whose
//! names are kept so they can be looked up.
//!
//! Starting the input unit is what makes macOS ask the user for the
//! microphone; nothing else here does.

use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use super::{
    AudioError, AudioIo, AudioOptions, Direction, InputCallback, OutputCallback, SAMPLE_RATE,
    StreamInfo,
};

#[allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
mod ffi {
    use std::ffi::{c_char, c_void};

    pub type OSStatus = i32;
    pub type AudioObjectID = u32;
    pub type AudioComponent = *mut c_void;
    pub type AudioUnit = *mut c_void;
    pub type CFStringRef = *const c_void;

    /// A four-character code, as Apple's headers write them: `'dOut'`.
    pub const fn fourcc(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    #[repr(C)]
    pub struct AudioObjectPropertyAddress {
        pub mSelector: u32,
        pub mScope: u32,
        pub mElement: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct AudioValueRange {
        pub mMinimum: f64,
        pub mMaximum: f64,
    }

    #[repr(C)]
    pub struct AudioComponentDescription {
        pub componentType: u32,
        pub componentSubType: u32,
        pub componentManufacturer: u32,
        pub componentFlags: u32,
        pub componentFlagsMask: u32,
    }

    #[repr(C)]
    pub struct AudioStreamBasicDescription {
        pub mSampleRate: f64,
        pub mFormatID: u32,
        pub mFormatFlags: u32,
        pub mBytesPerPacket: u32,
        pub mFramesPerPacket: u32,
        pub mBytesPerFrame: u32,
        pub mChannelsPerFrame: u32,
        pub mBitsPerChannel: u32,
        pub mReserved: u32,
    }

    #[repr(C)]
    pub struct AudioBuffer {
        pub mNumberChannels: u32,
        pub mDataByteSize: u32,
        pub mData: *mut c_void,
    }

    /// Declared with one buffer, which is all an interleaved or mono stream
    /// uses.
    #[repr(C)]
    pub struct AudioBufferList {
        pub mNumberBuffers: u32,
        pub mBuffers: [AudioBuffer; 1],
    }

    /// Only ever handled through a pointer, so its fields are not declared.
    #[repr(C)]
    pub struct AudioTimeStamp {
        _private: [u8; 0],
    }

    pub type AURenderCallback = extern "C" fn(
        inRefCon: *mut c_void,
        ioActionFlags: *mut u32,
        inTimeStamp: *const AudioTimeStamp,
        inBusNumber: u32,
        inNumberFrames: u32,
        ioData: *mut AudioBufferList,
    ) -> OSStatus;

    #[repr(C)]
    pub struct AURenderCallbackStruct {
        pub inputProc: AURenderCallback,
        pub inputProcRefCon: *mut c_void,
    }

    pub const kAudioObjectSystemObject: AudioObjectID = 1;
    pub const kAudioObjectPropertyScopeGlobal: u32 = fourcc(b"glob");
    pub const kAudioObjectPropertyScopeInput: u32 = fourcc(b"inpt");
    pub const kAudioObjectPropertyScopeOutput: u32 = fourcc(b"outp");
    pub const kAudioObjectPropertyElementMain: u32 = 0;
    pub const kAudioObjectPropertyName: u32 = fourcc(b"lnam");
    pub const kAudioHardwarePropertyDefaultInputDevice: u32 = fourcc(b"dIn ");
    pub const kAudioHardwarePropertyDefaultOutputDevice: u32 = fourcc(b"dOut");
    pub const kAudioDevicePropertyBufferFrameSize: u32 = fourcc(b"fsiz");
    pub const kAudioDevicePropertyBufferFrameSizeRange: u32 = fourcc(b"fsz#");
    pub const kAudioDevicePropertyLatency: u32 = fourcc(b"ltnc");
    pub const kAudioDevicePropertySafetyOffset: u32 = fourcc(b"saft");
    pub const kAudioDevicePropertyNominalSampleRate: u32 = fourcc(b"nsrt");
    pub const kAudioDevicePropertyStreams: u32 = fourcc(b"stm#");
    pub const kAudioDevicePropertyTransportType: u32 = fourcc(b"tran");
    pub const kAudioStreamPropertyLatency: u32 = fourcc(b"ltnc");
    pub const kAudioDeviceTransportTypeBluetooth: u32 = fourcc(b"blue");
    pub const kAudioDeviceTransportTypeBluetoothLE: u32 = fourcc(b"blea");

    pub const kAudioUnitType_Output: u32 = fourcc(b"auou");
    pub const kAudioUnitSubType_HALOutput: u32 = fourcc(b"ahal");
    pub const kAudioUnitManufacturer_Apple: u32 = fourcc(b"appl");
    pub const kAudioUnitScope_Global: u32 = 0;
    pub const kAudioUnitScope_Input: u32 = 1;
    pub const kAudioUnitScope_Output: u32 = 2;
    pub const kAudioUnitProperty_StreamFormat: u32 = 8;
    pub const kAudioUnitProperty_MaximumFramesPerSlice: u32 = 14;
    pub const kAudioUnitProperty_SetRenderCallback: u32 = 23;
    pub const kAudioOutputUnitProperty_CurrentDevice: u32 = 2000;
    pub const kAudioOutputUnitProperty_EnableIO: u32 = 2003;
    pub const kAudioOutputUnitProperty_SetInputCallback: u32 = 2005;
    /// AUComponent.h: a render asked for more than MaximumFramesPerSlice.
    pub const kAudioUnitErr_TooManyFramesToProcess: OSStatus = -10874;

    pub const kAudioFormatLinearPCM: u32 = fourcc(b"lpcm");
    pub const kAudioFormatFlagIsFloat: u32 = 1 << 0;
    pub const kAudioFormatFlagIsPacked: u32 = 1 << 3;

    pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        pub fn AudioObjectGetPropertyDataSize(
            inObjectID: AudioObjectID,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const c_void,
            outDataSize: *mut u32,
        ) -> OSStatus;
        pub fn AudioObjectGetPropertyData(
            inObjectID: AudioObjectID,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const c_void,
            ioDataSize: *mut u32,
            outData: *mut c_void,
        ) -> OSStatus;
        pub fn AudioObjectSetPropertyData(
            inObjectID: AudioObjectID,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const c_void,
            inDataSize: u32,
            inData: *const c_void,
        ) -> OSStatus;
    }

    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        pub fn AudioComponentFindNext(
            inComponent: AudioComponent,
            inDesc: *const AudioComponentDescription,
        ) -> AudioComponent;
        pub fn AudioComponentInstanceNew(
            inComponent: AudioComponent,
            outInstance: *mut AudioUnit,
        ) -> OSStatus;
        pub fn AudioComponentInstanceDispose(inInstance: AudioUnit) -> OSStatus;
        pub fn AudioUnitSetProperty(
            inUnit: AudioUnit,
            inID: u32,
            inScope: u32,
            inElement: u32,
            inData: *const c_void,
            inDataSize: u32,
        ) -> OSStatus;
        pub fn AudioUnitInitialize(inUnit: AudioUnit) -> OSStatus;
        pub fn AudioUnitUninitialize(inUnit: AudioUnit) -> OSStatus;
        pub fn AudioOutputUnitStart(ci: AudioUnit) -> OSStatus;
        pub fn AudioOutputUnitStop(ci: AudioUnit) -> OSStatus;
        pub fn AudioUnitRender(
            inUnit: AudioUnit,
            ioActionFlags: *mut u32,
            inTimeStamp: *const AudioTimeStamp,
            inOutputBusNumber: u32,
            inNumberFrames: u32,
            ioData: *mut AudioBufferList,
        ) -> OSStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFStringGetCString(
            theString: CFStringRef,
            buffer: *mut c_char,
            bufferSize: isize,
            encoding: u32,
        ) -> u8;
        pub fn CFRelease(cf: *const c_void);
    }
}

use ffi::*;

/// The largest callback we prepare buffers for. Core Audio asks for one
/// I/O buffer at a time, a few hundred frames at most; more only happens
/// when resampling, and is handled by growing the buffer.
const MAX_FRAMES: usize = 4096;
/// Playback runs in stereo: a mono stream would play in the left ear only
/// on some devices, so the backend copies each sample to both channels.
const OUTPUT_CHANNELS: usize = 2;

fn check(status: OSStatus, what: &str) -> Result<(), AudioError> {
    if status == 0 {
        Ok(())
    } else {
        Err(AudioError(format!(
            "Core Audio: {what} failed ({})",
            describe(status)
        )))
    }
}

/// Core Audio errors are often four-character codes; show them as such.
fn describe(status: OSStatus) -> String {
    let bytes = status.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    }
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Reads a fixed-size property of an audio object.
fn get<T: Default>(object: AudioObjectID, selector: u32, scope: u32) -> Result<T, AudioError> {
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` is a valid, writable T and `size` says how big it is;
    // Core Audio writes at most `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &address(selector, scope),
            0,
            ptr::null(),
            &mut size,
            (&mut value as *mut T).cast(),
        )
    };
    check(status, "reading a device property")?;
    Ok(value)
}

/// Writes a fixed-size property of an audio object.
fn set<T>(object: AudioObjectID, selector: u32, scope: u32, value: &T) -> Result<(), AudioError> {
    // SAFETY: `value` points to a valid T of the size passed.
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            &address(selector, scope),
            0,
            ptr::null(),
            size_of::<T>() as u32,
            (value as *const T).cast(),
        )
    };
    check(status, "setting a device property")
}

/// The device's name, or a placeholder if it has none.
fn device_name(device: AudioObjectID) -> String {
    let mut name: CFStringRef = ptr::null();
    let mut size = size_of::<CFStringRef>() as u32;
    // SAFETY: the name property is a CFStringRef, which we own afterwards.
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &address(kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal),
            0,
            ptr::null(),
            &mut size,
            (&mut name as *mut CFStringRef).cast(),
        )
    };
    if status != 0 || name.is_null() {
        return format!("device {device}");
    }
    let mut buffer = [0 as c_char; 256];
    // SAFETY: `buffer` is writable for its length; `name` is a valid
    // CFString that we release exactly once.
    let ok = unsafe {
        let ok = CFStringGetCString(
            name,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
            kCFStringEncodingUTF8,
        );
        CFRelease(name);
        ok
    };
    if ok == 0 {
        return format!("device {device}");
    }
    // SAFETY: CFStringGetCString succeeded, so `buffer` holds a
    // NUL-terminated string.
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// The latency Core Audio reports for the first stream of `device` in that
/// direction, which comes on top of the device's own latency.
fn stream_latency(device: AudioObjectID, scope: u32) -> u32 {
    let addr = address(kAudioDevicePropertyStreams, scope);
    let mut size = 0u32;
    // SAFETY: asks only for the size of the stream list.
    let status =
        unsafe { AudioObjectGetPropertyDataSize(device, &addr, 0, ptr::null(), &mut size) };
    if status != 0 || size < size_of::<AudioObjectID>() as u32 {
        return 0;
    }
    let mut streams = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
    // SAFETY: `streams` is writable for `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &addr,
            0,
            ptr::null(),
            &mut size,
            streams.as_mut_ptr().cast(),
        )
    };
    if status != 0 {
        return 0;
    }
    get::<u32>(
        streams[0],
        kAudioStreamPropertyLatency,
        kAudioObjectPropertyScopeGlobal,
    )
    .unwrap_or(0)
}

fn scope_of(direction: Direction) -> u32 {
    match direction {
        Direction::Input => kAudioObjectPropertyScopeInput,
        Direction::Output => kAudioObjectPropertyScopeOutput,
    }
}

/// The device macOS currently uses by default for a direction.
fn default_device(direction: Direction) -> Result<AudioObjectID, AudioError> {
    let selector = match direction {
        Direction::Input => kAudioHardwarePropertyDefaultInputDevice,
        Direction::Output => kAudioHardwarePropertyDefaultOutputDevice,
    };
    let device: AudioObjectID = get(
        kAudioObjectSystemObject,
        selector,
        kAudioObjectPropertyScopeGlobal,
    )?;
    if device == 0 {
        let name = match direction {
            Direction::Input => "input",
            Direction::Output => "output",
        };
        return Err(AudioError(format!("no default {name} device")));
    }
    Ok(device)
}

/// The smallest and largest I/O buffers the device accepts.
fn buffer_range(device: AudioObjectID, direction: Direction) -> Result<(u32, u32), AudioError> {
    let range: AudioValueRange = get(
        device,
        kAudioDevicePropertyBufferFrameSizeRange,
        scope_of(direction),
    )?;
    let min = range.mMinimum.max(1.0) as u32;
    Ok((min, (range.mMaximum as u32).max(min)))
}

/// The buffer `start` asks for: the one in the options, within the
/// device's range, or the smallest.
fn wanted_buffer(
    device: AudioObjectID,
    direction: Direction,
    options: AudioOptions,
) -> Result<u32, AudioError> {
    let (min, max) = buffer_range(device, direction)?;
    Ok(options.buffer_frames.unwrap_or(min).clamp(min, max))
}

/// Everything Core Audio reports about a device in one direction, as it
/// runs now.
fn describe_device(device: AudioObjectID, direction: Direction) -> Result<StreamInfo, AudioError> {
    let scope = scope_of(direction);
    let global = kAudioObjectPropertyScopeGlobal;
    let rate: f64 = get(device, kAudioDevicePropertyNominalSampleRate, global)?;
    let latency: u32 = get(device, kAudioDevicePropertyLatency, scope).unwrap_or(0);
    let safety: u32 = get(device, kAudioDevicePropertySafetyOffset, scope).unwrap_or(0);
    let stream = stream_latency(device, scope);
    let transport: u32 = get(device, kAudioDevicePropertyTransportType, global).unwrap_or(0);
    Ok(StreamInfo {
        direction,
        device: device_name(device),
        mode: "Core Audio HAL output unit".into(),
        device_rate: rate.round() as u32,
        buffer_frames: get(device, kAudioDevicePropertyBufferFrameSize, scope)?,
        min_buffer_frames: buffer_range(device, direction)?.0,
        latency_frames: latency + safety + stream,
        latency_parts: vec![
            ("device", latency),
            ("safety offset", safety),
            ("stream", stream),
        ],
        bluetooth: transport == kAudioDeviceTransportTypeBluetooth
            || transport == kAudioDeviceTransportTypeBluetoothLE,
    })
}

/// Reports what the default devices would give, without opening any
/// stream or changing any setting: no sound, and no microphone permission
/// prompt. The buffer shown is the one [`CoreAudio::start`] would ask for.
pub fn probe(options: AudioOptions) -> Vec<Result<StreamInfo, AudioError>> {
    [Direction::Input, Direction::Output]
        .into_iter()
        .map(|direction| {
            let device = default_device(direction)?;
            let mut info = describe_device(device, direction)?;
            info.buffer_frames = wanted_buffer(device, direction, options)?;
            Ok(info)
        })
        .collect()
}

/// What a callback needs, behind the pointer Core Audio hands back to it.
struct InputState {
    unit: AudioUnit,
    buffer: Vec<f32>,
    callback: InputCallback,
}

struct OutputState {
    mono: Vec<f32>,
    callback: OutputCallback,
}

/// One HAL output unit, disposed when dropped.
struct Unit(AudioUnit);

impl Unit {
    fn new() -> Result<Unit, AudioError> {
        let description = AudioComponentDescription {
            componentType: kAudioUnitType_Output,
            componentSubType: kAudioUnitSubType_HALOutput,
            componentManufacturer: kAudioUnitManufacturer_Apple,
            componentFlags: 0,
            componentFlagsMask: 0,
        };
        // SAFETY: a valid description; a null start searches from the first
        // component.
        let component = unsafe { AudioComponentFindNext(ptr::null_mut(), &description) };
        if component.is_null() {
            return Err(AudioError("Core Audio: no HAL output unit".into()));
        }
        let mut unit: AudioUnit = ptr::null_mut();
        // SAFETY: `component` is valid and `unit` is a valid place to write.
        let status = unsafe { AudioComponentInstanceNew(component, &mut unit) };
        check(status, "creating an audio unit")?;
        Ok(Unit(unit))
    }

    fn set<T>(&self, property: u32, scope: u32, element: u32, value: &T) -> Result<(), AudioError> {
        // SAFETY: the unit is valid and `value` points to a T of the size
        // passed; the unit copies it.
        let status = unsafe {
            AudioUnitSetProperty(
                self.0,
                property,
                scope,
                element,
                (value as *const T).cast(),
                size_of::<T>() as u32,
            )
        };
        check(status, "setting an audio unit property")
    }
}

impl Drop for Unit {
    fn drop(&mut self) {
        // SAFETY: the unit is valid; stopping or uninitializing a unit that
        // never started is allowed. It is disposed exactly once.
        unsafe {
            AudioOutputUnitStop(self.0);
            AudioUnitUninitialize(self.0);
            AudioComponentInstanceDispose(self.0);
        }
    }
}

/// Our side of every stream: float samples, packed, at 48 kHz.
fn client_format(channels: u32) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: f64::from(SAMPLE_RATE),
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
        mBytesPerPacket: 4 * channels,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4 * channels,
        mChannelsPerFrame: channels,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

extern "C" fn input_proc(
    refcon: *mut c_void,
    flags: *mut u32,
    timestamp: *const AudioTimeStamp,
    _bus: u32,
    frames: u32,
    _data: *mut AudioBufferList,
) -> OSStatus {
    // SAFETY: `refcon` is the InputState that CoreAudio::start boxed, which
    // outlives the unit, and only this callback touches it.
    let state = unsafe { &mut *refcon.cast::<InputState>() };
    let frames = frames as usize;
    // MaximumFramesPerSlice caps a callback at the buffer's size; growing
    // it here would allocate on the real-time thread.
    if frames > state.buffer.len() {
        return kAudioUnitErr_TooManyFramesToProcess;
    }
    let mut list = AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [AudioBuffer {
            mNumberChannels: 1,
            mDataByteSize: (frames * 4) as u32,
            mData: state.buffer.as_mut_ptr().cast(),
        }],
    };
    // SAFETY: an input callback fetches its samples by rendering bus 1 of
    // its own unit, with the flags and timestamp it was given, into a
    // buffer large enough for `frames` mono floats.
    let status =
        unsafe { AudioUnitRender(state.unit, flags, timestamp, 1, frames as u32, &mut list) };
    if status == 0 {
        (state.callback)(&state.buffer[..frames]);
    }
    status
}

extern "C" fn output_proc(
    refcon: *mut c_void,
    _flags: *mut u32,
    _timestamp: *const AudioTimeStamp,
    _bus: u32,
    frames: u32,
    data: *mut AudioBufferList,
) -> OSStatus {
    // SAFETY: as for input_proc, with the OutputState.
    let state = unsafe { &mut *refcon.cast::<OutputState>() };
    // SAFETY: Core Audio passes a buffer list in the stream format we set:
    // one interleaved buffer of `frames` stereo floats.
    let out = unsafe {
        let buffer = &mut (*data).mBuffers[0];
        let len = (buffer.mDataByteSize as usize / 4).min(frames as usize * OUTPUT_CHANNELS);
        std::slice::from_raw_parts_mut(buffer.mData.cast::<f32>(), len)
    };
    let (frames_out, _) = out.as_chunks_mut::<OUTPUT_CHANNELS>();
    // MaximumFramesPerSlice keeps a callback within one buffer of `mono`;
    // should it not, the rest is rendered in more passes rather than by
    // growing the buffer, which would allocate on the real-time thread.
    for pairs in frames_out.chunks_mut(state.mono.len()) {
        let mono = &mut state.mono[..pairs.len()];
        (state.callback)(mono);
        for (pair, sample) in pairs.iter_mut().zip(mono.iter()) {
            pair.fill(*sample);
        }
    }
    0
}

/// The default devices of this Mac. See the module documentation.
pub struct CoreAudio {
    options: AudioOptions,
    // Units are declared before the states so they stop and are disposed
    // first: their callbacks hold pointers into the states.
    input_unit: Option<Unit>,
    output_unit: Option<Unit>,
    input_state: Option<Box<InputState>>,
    output_state: Option<Box<OutputState>>,
}

// SAFETY: the units and states are only touched from the thread that owns
// this value, and by Core Audio's callbacks, which the units stop before
// anything is freed.
unsafe impl Send for CoreAudio {}

impl CoreAudio {
    /// The backend, not started yet.
    pub fn new(options: AudioOptions) -> CoreAudio {
        CoreAudio {
            options,
            input_unit: None,
            output_unit: None,
            input_state: None,
            output_state: None,
        }
    }

    /// Starts playback alone, on the default output device. [`AudioIo::start`]
    /// calls it; `drywire-cli devices --play-silence` uses it to check the
    /// output path without opening the microphone.
    pub fn start_output(&mut self, callback: OutputCallback) -> Result<StreamInfo, AudioError> {
        if self.output_unit.is_some() {
            return Err(AudioError("output already started".into()));
        }
        let device = default_device(Direction::Output)?;
        let buffer = wanted_buffer(device, Direction::Output, self.options)?;
        set(
            device,
            kAudioDevicePropertyBufferFrameSize,
            kAudioObjectPropertyScopeOutput,
            &buffer,
        )?;
        let unit = Unit::new()?;
        unit.set(
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            &device,
        )?;
        unit.set(
            kAudioUnitProperty_StreamFormat,
            kAudioUnitScope_Input,
            0,
            &client_format(OUTPUT_CHANNELS as u32),
        )?;
        unit.set(
            kAudioUnitProperty_MaximumFramesPerSlice,
            kAudioUnitScope_Global,
            0,
            &(MAX_FRAMES as u32),
        )?;
        let mut state = Box::new(OutputState {
            mono: vec![0.0; MAX_FRAMES],
            callback,
        });
        let render = AURenderCallbackStruct {
            inputProc: output_proc,
            inputProcRefCon: (&mut *state as *mut OutputState).cast(),
        };
        unit.set(
            kAudioUnitProperty_SetRenderCallback,
            kAudioUnitScope_Input,
            0,
            &render,
        )?;
        // SAFETY: the unit is fully configured; its callback's state is
        // stored in self below, before this function returns.
        let status = unsafe { AudioUnitInitialize(unit.0) };
        check(status, "initializing the output unit")?;
        self.output_state = Some(state);
        // SAFETY: as above.
        let status = unsafe { AudioOutputUnitStart(unit.0) };
        check(status, "starting the output unit")?;
        self.output_unit = Some(unit);
        // Read back what the device runs with now.
        describe_device(device, Direction::Output)
    }

    fn start_input(&mut self, callback: InputCallback) -> Result<StreamInfo, AudioError> {
        let device = default_device(Direction::Input)?;
        let rate: f64 = get(
            device,
            kAudioDevicePropertyNominalSampleRate,
            kAudioObjectPropertyScopeGlobal,
        )?;
        if rate.round() as u32 != SAMPLE_RATE {
            return Err(AudioError(format!(
                "the microphone {:?} runs at {rate} Hz; Drywire captures at {SAMPLE_RATE} Hz \
                 and Core Audio does not convert input rates: set it to 48000 Hz in \
                 Audio MIDI Setup",
                device_name(device)
            )));
        }
        let buffer = wanted_buffer(device, Direction::Input, self.options)?;
        set(
            device,
            kAudioDevicePropertyBufferFrameSize,
            kAudioObjectPropertyScopeInput,
            &buffer,
        )?;
        let unit = Unit::new()?;
        // Capture on element 1, no playback on element 0. EnableIO comes
        // before choosing the device.
        unit.set(
            kAudioOutputUnitProperty_EnableIO,
            kAudioUnitScope_Input,
            1,
            &1u32,
        )?;
        unit.set(
            kAudioOutputUnitProperty_EnableIO,
            kAudioUnitScope_Output,
            0,
            &0u32,
        )?;
        unit.set(
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            &device,
        )?;
        unit.set(
            kAudioUnitProperty_StreamFormat,
            kAudioUnitScope_Output,
            1,
            &client_format(1),
        )?;
        unit.set(
            kAudioUnitProperty_MaximumFramesPerSlice,
            kAudioUnitScope_Global,
            0,
            &(MAX_FRAMES as u32),
        )?;
        let mut state = Box::new(InputState {
            unit: unit.0,
            buffer: vec![0.0; MAX_FRAMES],
            callback,
        });
        let input = AURenderCallbackStruct {
            inputProc: input_proc,
            inputProcRefCon: (&mut *state as *mut InputState).cast(),
        };
        unit.set(
            kAudioOutputUnitProperty_SetInputCallback,
            kAudioUnitScope_Global,
            0,
            &input,
        )?;
        // SAFETY: as in start_output.
        let status = unsafe { AudioUnitInitialize(unit.0) };
        check(status, "initializing the input unit")?;
        self.input_state = Some(state);
        // SAFETY: as above. This is the call that may show the microphone
        // permission prompt.
        let status = unsafe { AudioOutputUnitStart(unit.0) };
        check(status, "starting the input unit")?;
        self.input_unit = Some(unit);
        describe_device(device, Direction::Input)
    }
}

impl AudioIo for CoreAudio {
    fn start(
        &mut self,
        input: InputCallback,
        output: OutputCallback,
    ) -> Result<Vec<StreamInfo>, AudioError> {
        if self.input_unit.is_some() || self.output_unit.is_some() {
            return Err(AudioError("already started".into()));
        }
        let result = self
            .start_output(output)
            .and_then(|out| Ok(vec![self.start_input(input)?, out]));
        if result.is_err() {
            self.stop();
        }
        result
    }

    fn stop(&mut self) {
        // Dropping a unit stops it; only then may its state go.
        self.input_unit = None;
        self.output_unit = None;
        self.input_state = None;
        self.output_state = None;
    }
}

impl Drop for CoreAudio {
    fn drop(&mut self) {
        self.stop();
    }
}
