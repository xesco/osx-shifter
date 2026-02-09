use std::sync::Arc;

use anyhow::{anyhow, Result};
use coreaudio::audio_unit::audio_format::LinearPcmFlags;
use coreaudio::audio_unit::macos_helpers::audio_unit_from_device_id;
use coreaudio::audio_unit::render_callback::{self, data};
use coreaudio::audio_unit::{AudioUnit, Element, SampleFormat, Scope, StreamFormat};

use crate::audio::ring_buffer::AudioRingBuffer;
use crate::config::CliArgs;
use crate::playback::controller::PlaybackController;
use crate::playback::state::PlaybackState;

mod coreaudio_device {
    use coreaudio_sys::*;
    use std::os::raw::c_void;

    pub type AudioDeviceID = u32;

    fn get_device_id(selector: u32) -> Option<AudioDeviceID> {
        let address = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut device_id: AudioDeviceID = 0;
        let mut size = std::mem::size_of::<AudioDeviceID>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut device_id as *mut _ as *mut c_void,
            )
        };
        if status == 0 && device_id != 0 {
            Some(device_id)
        } else {
            None
        }
    }

    fn get_device_name(device_id: AudioDeviceID) -> Option<String> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioObjectPropertyName,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut name_ref: CFStringRef = std::ptr::null();
        let mut size = std::mem::size_of::<CFStringRef>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut name_ref as *mut _ as *mut c_void,
            )
        };
        if status != 0 || name_ref.is_null() {
            return None;
        }
        let mut buf = [0i8; 256];
        let ok = unsafe {
            CFStringGetCString(
                name_ref,
                buf.as_mut_ptr(),
                buf.len() as CFIndex,
                kCFStringEncodingUTF8,
            )
        };
        unsafe { CFRelease(name_ref as *const c_void) };
        if ok != 0 {
            let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
            cstr.to_str().ok().map(|s| s.to_owned())
        } else {
            None
        }
    }

    /// Returns the ID of the "system output" device (physical speakers).
    pub fn system_output_device_id() -> Option<AudioDeviceID> {
        get_device_id(kAudioHardwarePropertyDefaultSystemOutputDevice)
    }

    /// Returns the name of the "system output" device (physical speakers).
    pub fn system_output_device_name() -> Option<String> {
        let id = system_output_device_id()?;
        get_device_name(id)
    }

    /// Returns (default_output_id, system_output_id) for device listing annotations.
    pub fn default_device_ids() -> (Option<AudioDeviceID>, Option<AudioDeviceID>) {
        let default = get_device_id(kAudioHardwarePropertyDefaultOutputDevice);
        let system = get_device_id(kAudioHardwarePropertyDefaultSystemOutputDevice);
        (default, system)
    }

    pub fn default_output_device_id() -> Option<AudioDeviceID> {
        get_device_id(kAudioHardwarePropertyDefaultOutputDevice)
    }

    fn get_all_device_ids() -> Vec<AudioDeviceID> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                kAudioObjectSystemObject,
                &address,
                0,
                std::ptr::null(),
                &mut size,
            )
        };
        if status != 0 || size == 0 {
            return Vec::new();
        }
        let count = size as usize / std::mem::size_of::<AudioDeviceID>();
        let mut device_ids = vec![0u32; count];
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                device_ids.as_mut_ptr() as *mut c_void,
            )
        };
        if status != 0 {
            Vec::new()
        } else {
            device_ids
        }
    }

    pub fn get_channel_count(device_id: AudioDeviceID, scope: u32) -> u32 {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyStreamConfiguration,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(device_id, &address, 0, std::ptr::null(), &mut size)
        };
        if status != 0 || size == 0 {
            return 0;
        }
        let mut buf = vec![0u8; size as usize];
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                buf.as_mut_ptr() as *mut c_void,
            )
        };
        if status != 0 {
            return 0;
        }
        let list = buf.as_ptr() as *const AudioBufferList;
        let num_buffers = unsafe { (*list).mNumberBuffers };
        let mut channels = 0u32;
        let buffers_ptr = unsafe { (*list).mBuffers.as_ptr() };
        for i in 0..num_buffers as usize {
            let ab = unsafe { &*buffers_ptr.add(i) };
            channels += ab.mNumberChannels;
        }
        channels
    }

    pub fn get_sample_rate(device_id: AudioDeviceID) -> u32 {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyNominalSampleRate,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut rate: f64 = 0.0;
        let mut size = std::mem::size_of::<f64>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                &mut rate as *mut _ as *mut c_void,
            )
        };
        if status == 0 {
            rate as u32
        } else {
            0
        }
    }

    pub struct DeviceInfo {
        pub id: AudioDeviceID,
        pub name: String,
        pub input_channels: u32,
        pub output_channels: u32,
        pub sample_rate: u32,
    }

    pub fn all_devices() -> Vec<DeviceInfo> {
        get_all_device_ids()
            .into_iter()
            .filter_map(|id| {
                let name = get_device_name(id)?;
                let input_channels = get_channel_count(id, kAudioObjectPropertyScopeInput);
                let output_channels = get_channel_count(id, kAudioObjectPropertyScopeOutput);
                let sample_rate = get_sample_rate(id);
                Some(DeviceInfo {
                    id,
                    name,
                    input_channels,
                    output_channels,
                    sample_rate,
                })
            })
            .collect()
    }

    /// Find a device ID by case-insensitive name substring.
    pub fn device_id_by_name(name: &str) -> Option<(AudioDeviceID, String)> {
        let lower = name.to_lowercase();
        for id in get_all_device_ids() {
            if let Some(dev_name) = get_device_name(id) {
                if dev_name.to_lowercase().contains(&lower) {
                    return Some((id, dev_name));
                }
            }
        }
        None
    }
}

const VIRTUAL_DEVICE_NAMES: &[&str] = &["blackhole", "soundflower", "loopback"];

fn is_virtual_device(name: &str) -> bool {
    let lower = name.to_lowercase();
    VIRTUAL_DEVICE_NAMES.iter().any(|v| lower.contains(v))
}

pub struct AudioEngine {
    _input_unit: AudioUnit,
    _output_unit: AudioUnit,
    pub controller: Arc<PlaybackController>,
    pub input_device_name: String,
    pub output_device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioEngine {
    pub fn new(args: &CliArgs) -> Result<Self> {
        // Find input device by name — must be a virtual device
        let (input_id, input_name) = coreaudio_device::device_id_by_name(&args.input_device)
            .ok_or_else(|| anyhow!("No audio device found matching '{}'", args.input_device))?;

        if !is_virtual_device(&input_name) {
            return Err(anyhow!(
                "'{input_name}' is not a virtual audio device.\n\
                 Use -l to list available input devices."
            ));
        }

        // Find output device — must be a physical (non-virtual) device
        let (output_id, output_name) = match &args.output_device {
            Some(name) => {
                let (id, dev_name) = coreaudio_device::device_id_by_name(name)
                    .ok_or_else(|| anyhow!("No audio device found matching '{name}'"))?;
                if is_virtual_device(&dev_name) {
                    return Err(anyhow!(
                        "'{dev_name}' is a virtual audio device and cannot be used as output.\n\
                         Use -l to list available output devices."
                    ));
                }
                if id == input_id {
                    return Err(anyhow!(
                        "Input and output cannot be the same device ('{dev_name}').\n\
                         Use -l to list available devices."
                    ));
                }
                (id, dev_name)
            }
            None => {
                // Try system output first (physical speakers even when default is virtual)
                if let Some(id) = coreaudio_device::system_output_device_id() {
                    let name = coreaudio_device::system_output_device_name()
                        .unwrap_or_else(|| "unknown".into());
                    (id, name)
                } else if let Some(id) = coreaudio_device::default_output_device_id() {
                    let name = coreaudio_device::all_devices()
                        .into_iter()
                        .find(|d| d.id == id)
                        .map(|d| d.name)
                        .unwrap_or_else(|| "unknown".into());
                    if is_virtual_device(&name) {
                        return Err(anyhow!(
                            "Default output device '{name}' is a virtual device.\n\
                             Use -o to specify a physical output device. Use -l to list available devices."
                        ));
                    }
                    (id, name)
                } else {
                    return Err(anyhow!("No default output device"));
                }
            }
        };

        // Get device properties
        let sample_rate = coreaudio_device::get_sample_rate(input_id);
        let channels = coreaudio_device::get_channel_count(
            input_id,
            coreaudio_sys::kAudioObjectPropertyScopeInput,
        ) as u16;

        if sample_rate == 0 || channels == 0 {
            return Err(anyhow!(
                "Could not determine sample rate or channels for '{input_name}'"
            ));
        }

        // Verify output sample rate matches
        let output_sr = coreaudio_device::get_sample_rate(output_id);
        if output_sr != sample_rate {
            return Err(anyhow!(
                "Sample rate mismatch: input ({input_name}) = {sample_rate}Hz, \
                 output ({output_name}) = {output_sr}Hz.\n\
                 Fix: Open Audio MIDI Setup and set both devices to the same sample rate."
            ));
        }

        let stream_format = StreamFormat {
            sample_rate: sample_rate as f64,
            sample_format: SampleFormat::F32,
            flags: LinearPcmFlags::IS_FLOAT | LinearPcmFlags::IS_PACKED,
            channels: channels as u32,
        };

        // Create ring buffer
        let capacity = sample_rate as usize * channels as usize * args.buffer_seconds as usize;
        let ring = Arc::new(AudioRingBuffer::new(capacity));

        // Create controller
        let controller = Arc::new(PlaybackController::new(
            ring.clone(),
            channels,
            sample_rate,
            args.latency_ms,
        ));

        // Set up input AudioUnit (capture from BlackHole)
        let mut input_unit = audio_unit_from_device_id(input_id, true)
            .map_err(|e| anyhow!("Failed to create input AudioUnit: {e}"))?;
        input_unit
            .set_stream_format(stream_format, Scope::Output, Element::Input)
            .map_err(|e| anyhow!("Failed to set input stream format: {e}"))?;

        let ring_input = ring.clone();
        type InputArgs = render_callback::Args<data::Interleaved<f32>>;
        input_unit
            .set_input_callback(move |args: InputArgs| {
                ring_input.write(args.data.buffer);
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to set input callback: {e}"))?;

        // Set up output AudioUnit (play to speakers)
        let mut output_unit = audio_unit_from_device_id(output_id, false)
            .map_err(|e| anyhow!("Failed to create output AudioUnit: {e}"))?;
        output_unit
            .set_stream_format(stream_format, Scope::Input, Element::Output)
            .map_err(|e| anyhow!("Failed to set output stream format: {e}"))?;

        let ctrl_output = controller.clone();
        let ch = channels;
        type OutputArgs = render_callback::Args<data::Interleaved<f32>>;
        output_unit
            .set_render_callback(move |args: OutputArgs| {
                let data = args.data.buffer;
                let frame_count = data.len() / ch as usize;
                let state = ctrl_output.pre_read(frame_count);

                if state == PlaybackState::Paused {
                    for s in data.iter_mut() {
                        *s = 0.0;
                    }
                } else {
                    ctrl_output.ring.read(data);
                }

                ctrl_output.apply_ramp(data);
                ctrl_output.apply_volume(data);
                ctrl_output.update_peaks(data);
                Ok(())
            })
            .map_err(|e| anyhow!("Failed to set output callback: {e}"))?;

        // Start both audio units
        input_unit
            .start()
            .map_err(|e| anyhow!("Failed to start input: {e}"))?;
        output_unit
            .start()
            .map_err(|e| anyhow!("Failed to start output: {e}"))?;

        Ok(Self {
            _input_unit: input_unit,
            _output_unit: output_unit,
            controller,
            input_device_name: input_name,
            output_device_name: output_name,
            sample_rate,
            channels,
        })
    }
}

pub fn list_all_devices(input_device: &str) -> Result<()> {
    let devices = coreaudio_device::all_devices();
    let (default_output_id, system_output_id) = coreaudio_device::default_device_ids();
    let input_id = coreaudio_device::device_id_by_name(input_device).map(|(id, _)| id);

    println!("Available input devices (virtual):");
    let mut found_virtual = false;
    for dev in &devices {
        if dev.input_channels == 0 {
            continue;
        }
        if !is_virtual_device(&dev.name) {
            continue;
        }
        found_virtual = true;
        println!(
            "  {}  [{}ch {}Hz]",
            dev.name, dev.input_channels, dev.sample_rate,
        );
    }
    if !found_virtual {
        println!("  (none found)");
    }

    println!("\nAvailable output devices:");
    for dev in &devices {
        if dev.output_channels == 0 {
            continue;
        }
        if Some(dev.id) == input_id {
            continue;
        }
        let mut tags = Vec::new();
        if Some(dev.id) == default_output_id {
            tags.push("default");
        }
        if Some(dev.id) == system_output_id && default_output_id != system_output_id {
            tags.push("system output");
        }
        let tag = if tags.is_empty() {
            String::new()
        } else {
            format!(" ({})", tags.join(", "))
        };
        println!(
            "  {}  [{}ch {}Hz]{tag}",
            dev.name, dev.output_channels, dev.sample_rate,
        );
    }

    Ok(())
}
