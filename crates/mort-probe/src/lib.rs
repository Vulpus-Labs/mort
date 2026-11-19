//! mort's CLAP analysis plugin (ADR 0005).
//!
//! A stereo effect that passes audio through unchanged. While the host
//! transport plays (live or rendering), it measures the audio with
//! `mort-features`, stamps each 50 ms frame with the project time of its
//! first sample and the bridge's run id, and writes it to a ring the bridge
//! owns. It finds the bridge through REAPER's `cockos.reaper_extension`; in
//! any other host, or with no bridge, it passes audio through and shows
//! `attached` as 0.
//!
//! The audio thread never allocates, locks or makes system calls: the
//! analyser is built on activate and the ring is fixed-size.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt::Write as _;
use std::sync::atomic::Ordering;

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams, PluginParams,
};
use clack_plugin::events::event_types::TransportFlags;
use clack_plugin::extensions::{Extension, HostExtensionSide, RawExtension};
use clack_plugin::prelude::*;
use clack_plugin::utils::Cookie;

use mort_features::Analyzer;
use mort_link::{ATTACH_API, AttachFn, LINK_VERSION, Record, Slot};

/// The plugin's CLAP id.
pub const ID: &str = "com.vulpuslabs.mort.probe";

const PARAM_SLOT: u32 = 0;
const PARAM_ATTACHED: u32 = 1;
/// How many times the host has reset the plugin (ADR 0012, ticket 0022).
const PARAM_RESETS: u32 = 2;

/// REAPER's `reaper_plugin_info_t`, which `cockos.reaper_extension` returns.
#[repr(C)]
#[derive(Clone, Copy)]
struct ReaperPluginInfo {
    caller_version: c_int,
    hwnd_main: *mut c_void,
    register: Option<unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int>,
    get_func: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
}

/// The `cockos.reaper_extension` host extension.
#[derive(Clone, Copy)]
struct ReaperExtension(RawExtension<HostExtensionSide, ReaperPluginInfo>);

// SAFETY: REAPER's extension struct is plain data and function pointers that
// may be used from any thread.
unsafe impl Send for ReaperExtension {}
// SAFETY: as above.
unsafe impl Sync for ReaperExtension {}

// SAFETY: `ReaperPluginInfo` matches the struct REAPER returns for this id.
unsafe impl Extension for ReaperExtension {
    const IDENTIFIERS: &[&CStr] = &[c"cockos.reaper_extension"];
    type ExtensionSide = HostExtensionSide;

    unsafe fn from_raw(raw: RawExtension<Self::ExtensionSide>) -> Self {
        // SAFETY: the caller guarantees the pointer is REAPER's struct.
        Self(unsafe { raw.cast() })
    }
}

/// Finds the bridge and attaches, if REAPER and the bridge are there.
fn attach(host: &HostSharedHandle<'_>) -> Option<&'static Slot> {
    let ext = host.info().get_extension::<ReaperExtension>()?;
    // SAFETY: REAPER keeps its info struct alive for the plugin's lifetime.
    let info = unsafe { ext.0.as_ptr().as_ref() };
    let get_func = info.get_func?;
    // SAFETY: GetFunc takes a NUL-terminated name and returns a pointer or null.
    let f = unsafe { get_func(ATTACH_API.as_ptr()) };
    if f.is_null() {
        return None;
    }
    // SAFETY: the bridge registered this name with this signature.
    let attach: AttachFn = unsafe { std::mem::transmute::<*mut c_void, AttachFn>(f) };
    // SAFETY: the bridge checks the version and size and returns a slot that
    // lives for the rest of the process, or null.
    let slot = unsafe { attach(LINK_VERSION, std::mem::size_of::<Slot>()) };
    // SAFETY: non-null slots live for the rest of the process.
    unsafe { slot.as_ref() }
}

/// The plugin.
pub struct Probe;

impl Plugin for Probe {
    type AudioProcessor<'a> = ProbeAudio<'a>;
    type Shared<'a> = ProbeShared;
    type MainThread<'a> = ProbeMain<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&ProbeShared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>();
    }
}

impl DefaultPluginFactory for Probe {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::{ANALYZER, AUDIO_EFFECT, STEREO, UTILITY};
        PluginDescriptor::new(ID, "mort probe")
            .with_vendor("Vulpus Labs")
            .with_version(env!("CARGO_PKG_VERSION"))
            .with_description("Measures the audio passing through it for mort's mix map")
            .with_features([AUDIO_EFFECT, ANALYZER, UTILITY, STEREO])
    }

    fn new_shared(host: HostSharedHandle<'_>) -> Result<ProbeShared, PluginError> {
        Ok(ProbeShared {
            slot: attach(&host),
            resets: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn new_main_thread<'a>(
        _host: HostMainThreadHandle<'a>,
        shared: &'a ProbeShared,
    ) -> Result<ProbeMain<'a>, PluginError> {
        Ok(ProbeMain { shared })
    }
}

/// Shared between threads: the slot, if attached.
pub struct ProbeShared {
    slot: Option<&'static Slot>,
    /// Times the host called `reset`, as on a seek it clears tails.
    resets: std::sync::atomic::AtomicU64,
}

impl PluginShared<'_> for ProbeShared {}

impl Drop for ProbeShared {
    fn drop(&mut self) {
        if let Some(s) = self.slot {
            s.attached.store(false, Ordering::Release);
        }
    }
}

/// Main-thread state.
pub struct ProbeMain<'a> {
    shared: &'a ProbeShared,
}

impl<'a> PluginMainThread<'a, ProbeShared> for ProbeMain<'a> {}

/// Measures blocks of audio and writes stamped frames to a slot. This is
/// all the probe does on the audio thread; it allocates only in `new`.
pub struct Meter {
    analyzer: Analyzer,
    rate: f64,
    /// Where the next block should start if playback is continuous.
    expected: Option<f64>,
}

impl Meter {
    /// A meter for `rate` Hz and blocks of up to `max_block` samples.
    ///
    /// # Errors
    ///
    /// If `mort-features` does not support the rate.
    pub fn new(rate: f64, max_block: usize) -> Result<Meter, mort_features::ConfigError> {
        Ok(Meter {
            analyzer: Analyzer::new(rate, max_block.max(1))?,
            rate,
            expected: None,
        })
    }

    /// Measures one block. `position` is the project time of its first
    /// sample while the transport plays, and `None` otherwise; nothing is
    /// measured then. A jump in the timeline starts the frames afresh.
    pub fn block(&mut self, position: Option<f64>, left: &[f32], right: &[f32], slot: &Slot) {
        let Some(pos) = position else {
            self.expected = None;
            return;
        };
        if self.expected.is_none_or(|e| (e - pos).abs() > 1e-3) {
            self.analyzer.reset();
        }
        let len = left.len().min(right.len());
        #[expect(clippy::cast_precision_loss, reason = "block lengths are small")]
        let block_seconds = len as f64 / self.rate;
        self.expected = Some(pos + block_seconds);
        let run = slot.run.load(Ordering::Relaxed);
        let rate = self.rate;
        self.analyzer.process(&left[..len], &right[..len], |frame| {
            let start = pos + (f64::from(frame.end_offset) - f64::from(frame.len)) / rate;
            slot.push(&Record {
                run,
                start,
                frame: *frame,
            });
        });
    }
}

/// Audio-thread state.
pub struct ProbeAudio<'a> {
    shared: &'a ProbeShared,
    meter: Meter,
}

impl<'a> PluginAudioProcessor<'a, ProbeShared, ProbeMain<'a>> for ProbeAudio<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main: &mut ProbeMain<'a>,
        shared: &'a ProbeShared,
        config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let max = usize::try_from(config.max_frames_count).unwrap_or(4096);
        let meter = Meter::new(config.sample_rate, max)
            .map_err(|_| PluginError::Message("unsupported sample rate"))?;
        if let Some(s) = shared.slot {
            s.sample_rate
                .store(config.sample_rate.to_bits(), Ordering::Relaxed);
            s.active.fetch_add(1, Ordering::Relaxed);
        }
        Ok(ProbeAudio { shared, meter })
    }

    fn reset(&mut self) {
        // Counted, not acted on: the meter restarts itself on any jump in
        // the timeline. The count shows whether REAPER resets plugins on a
        // seek (ADR 0012's open question).
        self.shared.resets.fetch_add(1, Ordering::Relaxed);
    }

    fn deactivate(self, _main: &mut ProbeMain<'a>) {
        if let Some(s) = self.shared.slot {
            s.active.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        _events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        let Some(mut port) = audio.port_pair(0) else {
            return Ok(ProcessStatus::Continue);
        };
        let Some(mut channels) = port.channels()?.into_f32() else {
            return Ok(ProcessStatus::Continue);
        };
        // Pass through, copying where input and output are separate, and
        // keep the first two channels for analysis.
        let mut bufs: [Option<&[f32]>; 2] = [None, None];
        for (pair, buf) in channels.iter_mut().zip(bufs.iter_mut()) {
            *buf = match pair {
                ChannelPair::InPlace(b) => Some(&*b),
                ChannelPair::InputOutput(i, o) => {
                    o.copy_from_slice(i);
                    Some(&*o)
                }
                ChannelPair::InputOnly(i) => Some(i),
                ChannelPair::OutputOnly(o) => {
                    o.fill(0.0);
                    Some(&*o)
                }
            };
        }
        let Some(slot) = self.shared.slot else {
            return Ok(ProcessStatus::Continue);
        };
        let position = process.transport.and_then(|t| {
            let playing = t.flags.contains(TransportFlags::IS_PLAYING);
            let timed = t.flags.contains(TransportFlags::HAS_SECONDS_TIMELINE);
            (playing && timed).then(|| t.song_pos_seconds.to_float())
        });
        if let (Some(l), Some(r)) = (bufs[0], bufs[1].or(bufs[0])) {
            self.meter.block(position, l, r, slot);
        }
        Ok(ProcessStatus::Continue)
    }
}

impl PluginAudioPortsImpl for ProbeMain<'_> {
    fn count(&mut self, _is_input: bool) -> u32 {
        1
    }

    fn get(&mut self, index: u32, _is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index == 0 {
            writer.set(&AudioPortInfo {
                id: ClapId::new(0),
                name: b"main",
                channel_count: 2,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(AudioPortType::STEREO),
                in_place_pair: Some(ClapId::new(0)),
            });
        }
    }
}

impl ProbeShared {
    fn slot_value(&self) -> f64 {
        self.slot.map_or(0.0, |s| f64::from(s.id))
    }

    fn attached_value(&self) -> f64 {
        if self
            .slot
            .is_some_and(|s| s.attached.load(Ordering::Relaxed))
        {
            1.0
        } else {
            0.0
        }
    }
}

impl PluginMainThreadParams for ProbeMain<'_> {
    fn count(&mut self) -> u32 {
        3
    }

    fn get_info(&mut self, index: u32, info: &mut ParamInfoWriter) {
        let flags = ParamInfoFlags::IS_READONLY | ParamInfoFlags::IS_STEPPED;
        match index {
            PARAM_SLOT => info.set(&ParamInfo {
                id: ClapId::new(PARAM_SLOT),
                flags,
                cookie: Cookie::default(),
                name: b"slot",
                module: b"",
                min_value: 0.0,
                max_value: f64::from(mort_link::SLOT_MAX),
                default_value: 0.0,
            }),
            PARAM_ATTACHED => info.set(&ParamInfo {
                id: ClapId::new(PARAM_ATTACHED),
                flags,
                cookie: Cookie::default(),
                name: b"attached",
                module: b"",
                min_value: 0.0,
                max_value: 1.0,
                default_value: 0.0,
            }),
            PARAM_RESETS => info.set(&ParamInfo {
                id: ClapId::new(PARAM_RESETS),
                flags,
                cookie: Cookie::default(),
                name: b"resets",
                module: b"",
                min_value: 0.0,
                max_value: 1_000_000.0,
                default_value: 0.0,
            }),
            _ => {}
        }
    }

    fn get_value(&mut self, id: ClapId) -> Option<f64> {
        match id.get() {
            PARAM_SLOT => Some(self.shared.slot_value()),
            PARAM_ATTACHED => Some(self.shared.attached_value()),
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            PARAM_RESETS => Some(self.shared.resets.load(Ordering::Relaxed) as f64),
            _ => None,
        }
    }

    fn value_to_text(
        &mut self,
        id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        match id.get() {
            PARAM_SLOT | PARAM_RESETS => write!(writer, "{value:.0}"),
            PARAM_ATTACHED => write!(writer, "{}", if value >= 0.5 { "yes" } else { "no" }),
            _ => Err(std::fmt::Error),
        }
    }

    fn text_to_value(&mut self, _id: ClapId, _text: &CStr) -> Option<f64> {
        None
    }

    fn flush(&mut self, _input: &InputEvents, _output: &mut OutputEvents) {}
}

impl PluginAudioProcessorParams for ProbeAudio<'_> {
    fn flush(&mut self, _input: &InputEvents, _output: &mut OutputEvents) {}
}

clack_export_entry!(SinglePluginEntry<Probe>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor() {
        let d = Probe::get_descriptor();
        assert_eq!(d.id().unwrap().to_str().unwrap(), ID);
        let _ = LINK_VERSION;
    }
}
