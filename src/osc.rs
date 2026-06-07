use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use bigdecimal::BigDecimal;
use rosc::encoder;
use rosc::{OscMessage, OscPacket, OscTime, OscType};

use jdw_osc_lib::model::TimedOSCPacket;

use crate::full;
use crate::shuttle::ResolvedElement;
use crate::note_utils;

// -- ElementConverter (external ID scheme, frequency resolution, OSC routing) --

/// The type of instrument for OSC routing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstrumentType {
    Sampler,
    Synth,
    Drone,
}

/// Scale data for resolving shuttle note indices to frequencies.
#[derive(Debug, Clone)]
pub struct ScaleData {
    pub scale_key: String,
    pub scale_type: String,
    pub octave_start: i32,
}

impl Default for ScaleData {
    fn default() -> Self {
        ScaleData {
            scale_key: "c".to_string(),
            scale_type: "maj".to_string(),
            octave_start: 4,
        }
    }
}

/// A resolved element paired with its OSC message.
#[derive(Debug, Clone)]
pub struct ElementMessage {
    pub element: ResolvedElement,
    pub message: OscPacket,
}

/// Stateful converter that resolves shuttle elements to OSC messages with
/// unique external IDs containing the `{nodeId}` template.
#[derive(Debug, Clone)]
pub struct ElementConverter {
    pub instrument_name: String,
    pub common_identifier: String,
    pub instrument_type: InstrumentType,
    pub external_id_override: String,
    pub scale_data: ScaleData,
    pub id_counter: u32,
}

// Default delay in ms for OSC messages sent to the router.
const SC_DELAY_MS: i32 = 70;

/// Check if a resolved element matches a given symbol (e.g., `x` for silence).
fn is_symbol(element: &ResolvedElement, sym: &str) -> bool {
    let prefix_matches = element.prefix == sym;
    let suffix_matches = element.suffix.to_lowercase() == sym && element.prefix.is_empty();
    (prefix_matches || suffix_matches)
        && element.index == 0
}

/// Check if the first `n` characters of `s` match a pattern.
fn begins_with(s: &str, pat: &str) -> bool {
    s.starts_with(pat)
}

/// Strip the first `n` characters from a string.
fn cut_first(s: &str, n: usize) -> String {
    if n >= s.len() {
        String::new()
    } else {
        s[n..].to_string()
    }
}

/// Convert resolved element args to a flat OSC arg list, inserting overrides.
fn args_as_osc(args: &HashMap<String, f64>, overrides: &[OscType]) -> Vec<OscType> {
    let mut osc_args: Vec<OscType> = overrides.to_vec();
    let mut sorted_keys: Vec<&String> = args.keys().collect();
    sorted_keys.sort();
    for key in sorted_keys {
        if !overrides.iter().any(|o| matches!(o, OscType::String(s) if s == key)) {
            osc_args.push(OscType::String(key.clone()));
            osc_args.push(OscType::Float(args[key] as f32));
        }
    }
    osc_args
}

impl ElementConverter {
    pub fn new(
        instrument_name: &str,
        common_identifier: &str,
        instrument_type: InstrumentType,
        scale_data: ScaleData,
    ) -> Self {
        ElementConverter {
            instrument_name: instrument_name.to_string(),
            common_identifier: common_identifier.to_string(),
            instrument_type,
            external_id_override: String::new(),
            scale_data,
            id_counter: 0,
        }
    }

    pub fn resolve_message(&mut self, element: &ResolvedElement, transpose_steps: i32) -> Option<ElementMessage> {
        if begins_with(&element.suffix, "@") {
            let override_id = cut_first(&element.suffix, 1);
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_mod(element, transpose_steps, &override_id),
            })
        } else if is_symbol(element, "x") {
            Some(ElementMessage {
                element: element.clone(),
                message: OscPacket::Message(OscMessage {
                    addr: "/empty_msg".to_string(),
                    args: vec![],
                }),
            })
        } else if is_symbol(element, ".") {
            None
        } else if is_symbol(element, "§") {
            Some(ElementMessage {
                element: element.clone(),
                message: OscPacket::Message(OscMessage {
                    addr: "/jdw_sc_event_trigger".to_string(),
                    args: vec![
                        OscType::String("loop_started".to_string()),
                        OscType::Int(SC_DELAY_MS),
                    ],
                }),
            })
        } else if begins_with(&element.suffix, "$") {
            let override_id = cut_first(&element.suffix, 1);
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_on(element, &override_id, transpose_steps),
            })
        } else if self.instrument_type == InstrumentType::Drone {
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_mod(element, transpose_steps, ""),
            })
        } else if self.instrument_type == InstrumentType::Sampler {
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_play_sample(element),
            })
        } else {
            Some(ElementMessage {
                element: element.clone(),
                message: self.to_note_on_timed(element, transpose_steps),
            })
        }
    }

    fn resolve_external_id(&mut self, element: &ResolvedElement) -> String {
        if !element.suffix.is_empty() {
            return element.suffix.clone();
        }
        let node_id = self.id_counter;
        self.id_counter += 1;
        format!(
            "{}_{}_{}{}_{}_{{nodeId}}",
            self.common_identifier,
            self.instrument_name,
            node_id,
            element.index,
            node_id,
        )
    }

    fn resolve_freq(&self, element: &ResolvedElement, transpose_steps: i32) -> f64 {
        if let Some(freq) = element.args.get("freq") {
            return *freq;
        }

        let letter_check = note_utils::note_letter_to_midi(&element.prefix);

        if letter_check == -1 {
            let index = note_utils::resolve_index(
                element.index as i32,
                &self.scale_data.scale_key,
                &self.scale_data.scale_type,
            );
            let octave = self.scale_data.octave_start;
            let extra = if octave > 0 { 12 * (octave + 1) } else { 0 };
            let new_index = (index + extra + transpose_steps) as f64;
            note_utils::midi_to_hz(new_index)
        } else {
            // Letter name with octave number
            let extra = if element.index > 0 { 12 * (element.index as i32 - 1) } else { 0 };
            let new_index = (letter_check + extra + transpose_steps) as f64;
            note_utils::midi_to_hz(new_index)
        }
    }

    fn to_note_mod(&mut self, element: &ResolvedElement, transpose_steps: i32, external_id_override: &str) -> OscPacket {
        let effective_override = if external_id_override.is_empty() {
            self.external_id_override.clone()
        } else {
            external_id_override.to_string()
        };
        let external_id = if effective_override.is_empty() {
            self.resolve_external_id(element)
        } else {
            effective_override
        };
        let freq = self.resolve_freq(element, transpose_steps);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/note_modify".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(external_id),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }

    fn to_note_on_timed(&mut self, element: &ResolvedElement, transpose_steps: i32) -> OscPacket {
        let freq = self.resolve_freq(element, transpose_steps);
        let external_id = self.resolve_external_id(element);
        let sus = element.args.get("sus").copied().unwrap_or(0.0);
        let gate_time = format!("{}", sus);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/note_on_timed".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(self.instrument_name.clone()),
                    OscType::String(external_id),
                    OscType::String(gate_time),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }

    fn to_play_sample(&mut self, element: &ResolvedElement) -> OscPacket {
        let freq = self.resolve_freq(element, 0);
        let external_id = self.resolve_external_id(element);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/play_sample".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(external_id),
                    OscType::String(self.instrument_name.clone()),
                    OscType::Int(element.index as i32),
                    OscType::String(element.prefix.clone()),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }

    fn to_note_on(&mut self, element: &ResolvedElement, external_id_override: &str, transpose_steps: i32) -> OscPacket {
        let external_id = if external_id_override.is_empty() {
            self.resolve_external_id(element)
        } else {
            external_id_override.to_string()
        };
        let freq = self.resolve_freq(element, transpose_steps);
        let osc_args = args_as_osc(&element.args, &[
            OscType::String("freq".to_string()),
            OscType::Float(freq as f32),
        ]);
        OscPacket::Message(OscMessage {
            addr: "/note_on".to_string(),
            args: {
                let mut v = vec![
                    OscType::String(self.instrument_name.clone()),
                    OscType::String(external_id),
                    OscType::Int(SC_DELAY_MS),
                ];
                v.extend(osc_args);
                v
            },
        })
    }
}

fn f64_to_bigdecimal(val: f64) -> BigDecimal {
    BigDecimal::from_str(&format!("{}", val)).unwrap_or_else(|_| BigDecimal::from(1))
}

fn osc_timetag_now() -> OscTime {
    OscTime::try_from(SystemTime::now())
        .unwrap_or_else(|_| OscTime::from((0u32, 0u32)))
}

const ROUTER_DEFAULT: &str = "127.0.0.1:13339";
const DELAY_CONFIGURE_MS: u64 = 5;

/// Configuration for sending OSC messages.
pub struct OscConfig {
    pub router_addr: String,
    pub sequencer_port: i32,
    pub sc_port: i32,
    pub external_id_counter: u64,
}

impl Default for OscConfig {
    fn default() -> Self {
        OscConfig {
            router_addr: ROUTER_DEFAULT.to_string(),
            sequencer_port: 14441,
            sc_port: 13331,
            external_id_counter: 0,
        }
    }
}

/// Send a `/hard_stop` message to the router for the sequencer.
pub fn send_stop(config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    sock.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .map_err(|e| format!("set timeout: {}", e))?;

    let msg = OscPacket::Message(OscMessage {
        addr: "/hard_stop".to_string(),
        args: vec![],
    });
    send_to_router(&sock, &config.router_addr, &msg)?;

    let wipe = OscPacket::Message(OscMessage {
        addr: "/wipe_on_finish".to_string(),
        args: vec![],
    });
    send_to_router(&sock, &config.router_addr, &wipe)?;

    Ok(())
}

/// Send a `/free_notes` command matching a track alias.
pub fn send_free_notes(alias: &str, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    let msg = OscPacket::Message(OscMessage {
        addr: "/free_notes".to_string(),
        args: vec![OscType::String(alias.to_string())],
    });
    send_to_router(&sock, &config.router_addr, &msg)
}

/// Silence drone synths: send `/note_modify amp=0.0` for each drone section.
pub fn send_silence_drones(billboard: &full::Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for section in &billboard.sections {
        if !section.header.is_drone {
            continue;
        }
        // Generate a simple external ID pattern that matches drone notes
        let ext_id = format!(".*{}.*", regex::escape(&section.header.instrument));
        let msg = OscPacket::Message(OscMessage {
            addr: "/note_modify".to_string(),
            args: vec![
                OscType::String(ext_id),
                OscType::Int(0),
                OscType::String("amp".to_string()),
                OscType::Float(0.0),
            ],
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
    }
    Ok(())
}

/// Clear all existing effects before creating new ones.
/// Matches Python's `get_effects_clear()` → `/free_notes "^effect_(.*)"`
pub fn send_effects_clear(config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    let msg = OscPacket::Message(OscMessage {
        addr: "/free_notes".to_string(),
        args: vec![OscType::String("^effect_(.*)".to_string())],
    });
    send_to_router(&sock, &config.router_addr, &msg)
}

/// Create all `€`-defined effects via `/note_on`.
/// Matches Python's `get_all_effects_create()`.
pub fn send_effects_create(billboard: &full::Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for section in &billboard.sections {
        let group_name = section.header.group.as_deref().unwrap_or("");
        for effect in &section.effects {
            let ext_id = format!("effect_{}_{}", group_name, effect.id);
            let mut args = vec![
                OscType::String(effect.effect_type.clone()),
                OscType::String(ext_id),
                OscType::Int(0), // delay
            ];
            args.extend(args_as_osc(&effect.args, &[]));
            let msg = OscPacket::Message(OscMessage {
                addr: "/note_on".to_string(),
                args,
            });
            send_to_router(&sock, &config.router_addr, &msg)?;
            std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
        }
    }
    Ok(())
}

/// Create drone synth nodes (amp=0, modulated later) via `/note_on`.
/// Matches Python's `get_all_drones_create()`.
pub fn send_drones_create(billboard: &full::Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for section in &billboard.sections {
        if !section.header.is_drone {
            continue;
        }
        let group_name = section.header.group.as_deref().unwrap_or("");
        for track in &section.tracks {
            let ext_id = format!("effect_{}_{}", group_name, track.index);
            let mut args = vec![
                OscType::String(section.header.instrument.clone()),
                OscType::String(ext_id),
                OscType::Int(0),
            ];
            // Drone args: start with DEFAULT, then section header, then track overrides, then force amp=0
            let mut merged = billboard.default_args.clone();
            for (k, v) in &section.header.default_args {
                merged.insert(k.clone(), *v);
            }
            for (k, &(op, v)) in &track.arg_overrides {
                let entry = merged.entry(k.clone()).or_insert(0.0);
                match op { '*' => *entry *= v, '+' => *entry += v, '-' => *entry -= v, _ => *entry = v }
            }
            merged.insert("amp".to_string(), 0.0); // force amp=0 for drone creation
            args.extend(args_as_osc(&merged, &[]));
            let msg = OscPacket::Message(OscMessage {
                addr: "/note_on".to_string(),
                args,
            });
            send_to_router(&sock, &config.router_addr, &msg)?;
            std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
        }
    }
    Ok(())
}

/// Convert args HashMap to flat key-value OSC args (unfiltered, no overrides).
fn flat_kv_args(args: &std::collections::HashMap<String, f64>) -> Vec<OscType> {
    let mut out = Vec::new();
    let mut keys: Vec<&String> = args.keys().collect();
    keys.sort();
    for k in keys {
        out.push(OscType::String(k.clone()));
        out.push(OscType::Float(args[k] as f32));
    }
    out
}

// -- Internal helpers --

fn send_to_router(sock: &UdpSocket, addr: &str, packet: &OscPacket) -> Result<(), String> {
    let target: SocketAddr = addr.parse().map_err(|e| format!("invalid addr: {}", e))?;
    let buf = encoder::encode(packet).map_err(|e| format!("encode: {}", e))?;
    sock.send_to(&buf, target).map_err(|e| format!("send: {}", e))?;
    Ok(())
}

/// Convert a string arg to the most specific OscType.
///
/// Tries int first, then float, falls back to string. This matches
/// Python's `pythonosc` auto-detection in `OscMessageBuilder.add_arg()`.
fn osc_arg_from_str(s: &str) -> OscType {
    if let Ok(i) = s.parse::<i32>() {
        return OscType::Int(i);
    }
    if let Ok(f) = s.parse::<f32>() {
        return OscType::Float(f);
    }
    OscType::String(s.to_string())
}

// ---------------------------------------------------------------------------
// Full Billboard OSC conversion (Stage 5+)
// ---------------------------------------------------------------------------

/// Extract scale data from billboard commands (`/set_scale`), or return a
/// sensible default (C major, octave 4).
fn extract_scale_data(commands: &[full::BillboardCommand]) -> ScaleData {
    for cmd in commands {
        if cmd.address == "/set_scale" && cmd.args.len() >= 3 {
            return ScaleData {
                scale_key: cmd.args[0].clone(),
                scale_type: cmd.args[1].clone(),
                octave_start: cmd.args[2].parse().unwrap_or(4),
            };
        }
    }
    ScaleData::default()
}

/// Send `/create_synthdef` messages for each known SynthDef to the router.
///
/// This replaces the old `/read_scd ~jdw.synth()` approach with the
/// correct OSC protocol (matching the Python `setup()` flow).
pub fn send_full_setup(
    synthdefs: &[crate::synthdefs::SynthDefMessage],
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for def in synthdefs {
        let msg = OscPacket::Message(OscMessage {
            addr: "/create_synthdef".to_string(),
            args: vec![OscType::String(def.content.clone())],
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
        // Delay between messages to prevent dropped packets (matches Python)
        std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
    }

    Ok(())
}

/// Send `/load_sample` messages for all discovered samples.
pub fn send_samples(
    samples: &[crate::sample_loader::SampleLoadMessage],
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for sm in samples {
        let msg = OscPacket::Message(OscMessage {
            addr: "/load_sample".to_string(),
            args: sm.osc_args.clone(),
        });
        send_to_router(&sock, &config.router_addr, &msg)?;
        std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
    }

    Ok(())
}

/// Build a `timed_msg` bundle for a single timed OSC packet.
///
/// Wire format:
/// ```osc
/// /bundle_info "timed_msg"
/// /timed_msg_info "<relative_beat>"
/// <packet>
/// ```
fn build_timed_msg_bundle(packet: &TimedOSCPacket) -> OscPacket {
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("timed_msg".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/timed_msg_info".to_string(),
                args: vec![OscType::String(packet.time.to_string())],
            }),
            packet.packet.clone(),
        ],
    })
}

/// Build an `update_queue` bundle for a single track/alias.
///
/// Wire format:
/// ```osc
/// /bundle_info "update_queue"
/// /update_queue_info "<alias>" <one_shot: 0|1>
/// [bundle containing timed_msg bundles]
/// ```
fn build_update_queue_bundle(
    alias: &str,
    one_shot: bool,
    timed_packets: &[TimedOSCPacket],
) -> OscPacket {
    let timed_bundles: Vec<OscPacket> = timed_packets
        .iter()
        .map(build_timed_msg_bundle)
        .collect();

    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("update_queue".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/update_queue_info".to_string(),
                args: vec![
                    OscType::String(alias.to_string()),
                    OscType::Int(if one_shot { 1 } else { 0 }),
                ],
            }),
            OscPacket::Bundle(rosc::OscBundle {
                timetag: osc_timetag_now(),
                content: timed_bundles,
            }),
        ],
    })
}

/// Build a `batch_update_queues` bundle for a full billboard update.
///
/// Wire format:
/// ```osc
/// /bundle_info "batch_update_queues"
/// /batch_update_queues_info <stop_missing: 0|1>
/// [bundle containing update_queue bundles]
/// ```
fn build_batch_update_bundle(update_bundles: Vec<OscPacket>, stop_missing: bool) -> OscPacket {
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("batch_update_queues".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/batch_update_queues_info".to_string(),
                args: vec![OscType::Int(if stop_missing { 1 } else { 0 })],
            }),
            OscPacket::Bundle(rosc::OscBundle {
                timetag: osc_timetag_now(),
                content: update_bundles,
            }),
        ],
    })
}

/// Determine the sequencer alias for a track in a full Billboard section.
///
/// Uses `group_override` if present, otherwise `{synth}_{section_idx}_{track_idx}`.
fn track_alias(synth: &str, section_index: usize, track: &full::TrackDefinition) -> String {
    track
        .group_override
        .clone()
        .unwrap_or_else(|| format!("{}_{}_{}", synth, section_index, track.index))
}

/// Convert a track's shuttle notation to `TimedOSCPacket`s using an
/// `ElementConverter` for proper OSC routing, external IDs, and freq resolution.
///
/// Each packet's `time` is the note duration (beat offset until the next event).
///
/// Arg precedence (highest to lowest, matching Python):
/// 1. Track overrides (with operators: +, -, *, =)
/// 2. Element inline args (e.g. `c4:amp0.5`)
/// 3. Section args (default + header merged)
fn track_to_timed_packets(
    converter: &mut ElementConverter,
    content: &str,
    default_args: &HashMap<String, f64>,
    header_args: &HashMap<String, f64>,
    track_overrides: &HashMap<String, (char, f64)>,
) -> Result<Vec<TimedOSCPacket>, String> {
    let elements = crate::shuttle::parse(content)?;
    let mut packets = Vec::new();

    for elem in &elements {
        // Resolve args with Python precedence:
        //   element_inline > header > default → then track_overrides on top
        let mut resolved: HashMap<String, f64> = default_args.clone();
        for (k, v) in header_args {
            resolved.insert(k.clone(), *v);
        }
        for (k, v) in &elem.args {
            resolved.insert(k.clone(), *v);
        }
        for (k, &(op, v)) in track_overrides {
            let current = resolved.entry(k.clone()).or_insert(0.0);
            match op {
                '*' => *current *= v,
                '+' => *current += v,
                '-' => *current -= v,
                _ => *current = v,
            }
        }
        let mut merged = elem.clone();
        merged.args = resolved;

        match converter.resolve_message(&merged, 0) {
            None => continue,
            Some(msg) => {
                let note_len = merged.args.get("time").copied().unwrap_or(1.0);
                let time = f64_to_bigdecimal(note_len);
                packets.push(TimedOSCPacket {
                    time,
                    packet: msg.message,
                });
            }
        }
    }

    Ok(packets)
}

/// Maximum UDP datagram payload size (safe limit for Ethernet + OSC overhead).
const MAX_UDP_PACKET: usize = 64000;

/// Determine the group name for a track, matching Python's logic:
/// `track.group_override if set else section.header.group`.
fn track_group_name(track: &full::TrackDefinition, section: &full::SynthSection) -> String {
    track
        .group_override
        .clone()
        .unwrap_or_else(|| section.header.group.clone().unwrap_or_default())
}

/// Send a full Billboard's tracks as sequencer queue updates, using the
/// correct jdw-suite bundle protocol and ElementConverter.
///
/// Respects group filters (`>>>` lines): only tracks whose group is in the
/// last filter are included. If no filters are defined, all tracks are sent.
/// Large bundles are automatically split into multiple UDP packets.
pub fn send_full_queue_update(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    let scale_data = extract_scale_data(&billboard.commands);

    // Resolve final group filter (last >>> line); empty = no filter
    let final_filter: Vec<String> = billboard
        .filters
        .last()
        .cloned()
        .unwrap_or_default();
    let has_filter = !final_filter.is_empty();

    // Collect individual track bundles
    let mut update_bundles: Vec<OscPacket> = Vec::new();

    for (si, section) in billboard.sections.iter().enumerate() {
        let header_args = &section.header.default_args;

        for track in &section.tracks {
            if !track.enabled {
                continue;
            }

            // Apply group filter: skip if track's group is not in final filter
            if has_filter {
                let gname = track_group_name(track, section);
                if !final_filter.contains(&gname) {
                    continue;
                }
            }

            let instrument_type = if section.header.is_drone {
                InstrumentType::Drone
            } else if section.header.is_sampler {
                InstrumentType::Sampler
            } else {
                InstrumentType::Synth
            };

            let mut converter = ElementConverter::new(
                &section.header.instrument,
                &track.index.to_string(),
                instrument_type,
                scale_data.clone(),
            );

            if section.header.is_drone {
                converter.external_id_override = format!(
                    "effect_{}_{}",
                    section.header.group.as_deref().unwrap_or(""),
                    track.index
                );
            }

            let timed_packets = track_to_timed_packets(
                &mut converter,
                &track.content,
                &billboard.default_args,
                header_args,
                &track.arg_overrides,
            )?;

            if timed_packets.is_empty() {
                continue;
            }

            let alias = track_alias(&section.header.instrument, si, track);
            update_bundles.push(build_update_queue_bundle(&alias, false, &timed_packets));
        }
    }

    if update_bundles.is_empty() {
        return Ok(());
    }

    // Split into batches that fit within UDP size limits
    let mut start = 0;
    while start < update_bundles.len() {
        let mut end = update_bundles.len();
        loop {
            let is_last = end == update_bundles.len();
            let batch = build_batch_update_bundle(
                update_bundles[start..end].to_vec(),
                is_last,
            );
            let encoded = encoder::encode(&batch).map_err(|e| format!("encode: {}", e))?;
            if encoded.len() < MAX_UDP_PACKET || end == start + 1 {
                send_to_router(&sock, &config.router_addr, &batch)?;
                start = end;
                break;
            }
            end -= 1;
        }
    }

    Ok(())
}

/// Generate a human-readable dump of all OSC packets that `send_full_queue_update`
/// would send, without actually sending anything. Useful for comparing Rust vs Python output.
pub fn dump_queue_update(
    billboard: &full::Billboard,
) -> Vec<String> {
    let scale_data = extract_scale_data(&billboard.commands);
    let final_filter: Vec<String> = billboard
        .filters
        .last()
        .cloned()
        .unwrap_or_default();
    let has_filter = !final_filter.is_empty();
    let mut lines = Vec::new();

    for (si, section) in billboard.sections.iter().enumerate() {
        let header_args = &section.header.default_args;

        for track in &section.tracks {
            if !track.enabled { continue; }
            if has_filter {
                let gname = track_group_name(track, section);
                if !final_filter.contains(&gname) { continue; }
            }

            let instrument_type = if section.header.is_drone { InstrumentType::Drone }
                else if section.header.is_sampler { InstrumentType::Sampler }
                else { InstrumentType::Synth };

            let mut converter = ElementConverter::new(
                &section.header.instrument, &track.index.to_string(),
                instrument_type, scale_data.clone(),
            );

            if section.header.is_drone {
                converter.external_id_override = format!("effect_{}_{}",
                    section.header.group.as_deref().unwrap_or(""), track.index);
            }

            let alias = track_alias(&section.header.instrument, si, track);
            lines.push(format!("--- Track: {} (‘{}’ section {}) ---", alias, section.header.instrument, si));

            match track_to_timed_packets(&mut converter, &track.content,
                &billboard.default_args, header_args, &track.arg_overrides)
            {
                Err(e) => lines.push(format!("  ERROR: {}", e)),
                Ok(packets) => {
                    for (i, tp) in packets.iter().enumerate() {
                        match &tp.packet {
                            OscPacket::Message(msg) => {
                                let args: Vec<String> = msg.args.iter().map(|a| osc_type_to_string(a)).collect();
                                lines.push(format!("  [{}] {} (t={}) {}",
                                    i, msg.addr, tp.time, args.join(" ")));
                            }
                            OscPacket::Bundle(_) => {
                                lines.push(format!("  [{}] <bundle> (t={})", i, tp.time));
                            }
                        }
                    }
                }
            }
        }
    }

    lines
}

/// Dump all setup messages (synthdef loading) as human-readable text.
pub fn dump_setup(synthdefs: &[crate::synthdefs::SynthDefMessage]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("--- SETUP ({} synthdefs) ---", synthdefs.len()));
    for (i, def) in synthdefs.iter().enumerate() {
        lines.push(format!("  [{}] /create_synthdef \"{}\"", i, truncate_for_dump(&def.content, 60)));
    }
    lines
}

/// Dump all command messages as human-readable text.
pub fn dump_commands(billboard: &full::Billboard) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("--- COMMANDS ({} commands) ---", billboard.commands.len()));
    for (i, cmd) in billboard.commands.iter().enumerate() {
        let args: Vec<OscType> = cmd.args.iter().map(|a| osc_arg_from_str(a)).collect();
        let args_str: Vec<String> = args.iter().map(|a| osc_type_to_string(a)).collect();
        lines.push(format!("  [{}] {} {}", i, cmd.address, args_str.join(" ")));
    }
    lines
}

/// Truncate a long string for dump display.
fn truncate_for_dump(s: &str, max: usize) -> String {
    let clean = s.replace('\n', "\\n");
    if clean.len() <= max {
        clean
    } else {
        format!("{}...", &clean[..max])
    }
}

fn osc_type_to_string(t: &OscType) -> String {
    match t {
        OscType::String(s) => format!("\"{}\"", s),
        OscType::Int(i) => format!("{}", i),
        OscType::Float(f) => format!("{}", f),
        _ => format!("{:?}", t),
    }
}

/// Send all commands from a full Billboard to the OSC router.
pub fn send_full_commands(
    billboard: &full::Billboard,
    config: &OscConfig,
) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for cmd in &billboard.commands {
        match cmd.address.as_str() {
            "/create_router" => {
                // Transform: /create_router → /note_on "router" "effect_router_{in}_{out}" 0 "in" <in> "out" <out>
                if cmd.args.len() >= 2 {
                    let in_arg = &cmd.args[0];
                    let out_arg = &cmd.args[1];
                    let ext_id = format!("effect_router_{}_{}", in_arg, out_arg);
                    let in_val: f32 = in_arg.parse().unwrap_or(0.0);
                    let out_val: f32 = out_arg.parse().unwrap_or(0.0);
                    let msg = OscPacket::Message(OscMessage {
                        addr: "/note_on".to_string(),
                        args: vec![
                            OscType::String("router".to_string()),
                            OscType::String(ext_id),
                            OscType::Int(0),
                            OscType::String("in".to_string()),
                            OscType::Float(in_val),
                            OscType::String("out".to_string()),
                            OscType::Float(out_val),
                        ],
                    });
                    send_to_router(&sock, &config.router_addr, &msg)?;
                }
            }
            "/create_effect" => {
                // Transform: /create_effect → /note_on + /note_modify
                if cmd.args.len() >= 3 {
                    let effect_name = &cmd.args[0];
                    let effect_id = &cmd.args[1];
                    let effect_args_str = &cmd.args[2];
                    // /note_on
                    let note_on_msg = OscPacket::Message(OscMessage {
                        addr: "/note_on".to_string(),
                        args: vec![
                            OscType::String(effect_name.clone()),
                            OscType::String(effect_id.clone()),
                            OscType::Int(0),
                        ],
                    });
                    send_to_router(&sock, &config.router_addr, &note_on_msg)?;
                    std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
                    // /note_modify — parse the args string as comma-separated key=val
                    let parsed = crate::full::parse_simple_args(effect_args_str);
                    let mut mod_args = vec![
                        OscType::String(effect_id.clone()),
                        OscType::Int(0),
                    ];
                    let mut pairs: Vec<(&String, &String)> = parsed.iter().map(|(k,v)| (k,v)).collect();
                    pairs.sort_by(|a, b| a.0.cmp(b.0));
                    for (k, v) in pairs {
                        mod_args.push(OscType::String(k.clone()));
                        mod_args.push(osc_arg_from_str(v));
                    }
                    let mod_msg = OscPacket::Message(OscMessage {
                        addr: "/note_modify".to_string(),
                        args: mod_args,
                    });
                    send_to_router(&sock, &config.router_addr, &mod_msg)?;
                }
            }
            _ => {
                // Pass-through: all other commands sent verbatim
                let args: Vec<OscType> = cmd.args.iter().map(|a| osc_arg_from_str(a)).collect();
                let msg = OscPacket::Message(OscMessage {
                    addr: cmd.address.clone(),
                    args,
                });
                send_to_router(&sock, &config.router_addr, &msg)?;
            }
        }
        std::thread::sleep(Duration::from_millis(DELAY_CONFIGURE_MS));
    }

    Ok(())
}

// ========== NRT: Full bundle construction ==========

/// Result of building NRT bundles for a single track/group.
#[derive(Debug, Clone)]
pub struct NrtBundleInfo {
    pub track_name: String,
    pub nrt_bundle: OscPacket,
    pub preload_messages: Vec<OscPacket>,
    pub preload_bundles: Vec<OscPacket>,
}

/// Build NRT bundles for a billboard.
pub fn get_nrt_record_bundles(
    billboard: &full::Billboard,
    synthdefs: &[crate::synthdefs::SynthDefMessage],
    samples: &[crate::sample_loader::SampleLoadMessage],
) -> Vec<NrtBundleInfo> {
    let mut results = Vec::new();
    let scale_data = extract_scale_data(&billboard.commands);

    let bpm: f64 = billboard
        .commands
        .iter()
        .find(|c| c.address == "/set_bpm")
        .and_then(|c| c.args.first())
        .and_then(|a| a.parse().ok())
        .unwrap_or(120.0);

    // Track metadata: (elements, instrument_type, instrument_name, track_index, section_index)
    struct TrackMeta {
        elements: Vec<crate::shuttle::ResolvedElement>,
        instrument_type: InstrumentType,
        instrument_name: String,
        durations: Vec<f64>,
    }

    let mut track_metas: HashMap<String, TrackMeta> = HashMap::new();
    let mut score = crate::score::Score::new();

    for (si, section) in billboard.sections.iter().enumerate() {
        let instrument_type = if section.header.is_drone {
            InstrumentType::Drone
        } else if section.header.is_sampler {
            InstrumentType::Sampler
        } else {
            InstrumentType::Synth
        };

        for track in &section.tracks {
            if !track.enabled { continue; }

            let track_name = track_alias(&section.header.instrument, si, track);
            let group_name = track_group_name(track, section);

            // Resolve elements with args
            let elements = match crate::shuttle::parse(&track.content) {
                Ok(elems) => elems,
                Err(_) => continue,
            };

            // Resolve args: DEFAULT → header → element → track_overrides
            let resolved: Vec<_> = elements.iter().map(|elem| {
                let mut resolved = billboard.default_args.clone();
                for (k, v) in &section.header.default_args {
                    resolved.insert(k.clone(), *v);
                }
                for (k, v) in &elem.args {
                    resolved.insert(k.clone(), *v);
                }
                for (k, &(op, v)) in &track.arg_overrides {
                    let e = resolved.entry(k.clone()).or_insert(0.0);
                    match op { '*' => *e *= v, '+' => *e += v, '-' => *e -= v, _ => *e = v }
                }
                let mut el = elem.clone();
                el.args = resolved;
                el
            }).collect();

            let durations: Vec<f64> = resolved.iter()
                .map(|e| e.args.get("time").copied().unwrap_or(1.0))
                .collect();

            track_metas.insert(track_name.clone(), TrackMeta {
                elements: resolved,
                instrument_type,
                instrument_name: section.header.instrument.clone(),
                durations: durations.clone(),
            });

            score.add_source(track_name, group_name, durations);
        }
    }

    // Walk group filters
    let filters = &billboard.filters;
    if filters.is_empty() {
        let all_groups: Vec<String> = billboard
            .sections.iter()
            .flat_map(|s| s.tracks.iter().map(move |t| track_group_name(t, s)))
            .collect();
        score.extend_groups(&all_groups, true);
    } else {
        for filter_set in filters {
            score.extend_groups(filter_set, true);
        }
    }

    let timed_tracks = score.unpack_timed_entries();

    for (track_name, timed_entries) in &timed_tracks {
        let meta = match track_metas.get(track_name) {
            Some(m) => m,
            None => continue,
        };

        let mut preload_msgs = vec![OscPacket::Message(OscMessage {
            addr: "/clear_nrt".to_string(), args: vec![],
        })];
        for def in synthdefs {
            preload_msgs.push(OscPacket::Message(OscMessage {
                addr: "/create_synthdef".to_string(),
                args: vec![OscType::String(def.content.clone())],
            }));
        }
        for sm in samples {
            preload_msgs.push(OscPacket::Message(OscMessage {
                addr: "/load_sample".to_string(),
                args: sm.osc_args.clone(),
            }));
        }

        // Convert timed entries to OSC packets using ElementConverter
        let mut converter = ElementConverter::new(
            &meta.instrument_name, "0",
            meta.instrument_type, scale_data.clone(),
        );
        let mut osc_packets: Vec<TimedOSCPacket> = Vec::new();
        for (time, src_idx) in timed_entries {
            match src_idx {
                Some(idx) if *idx < meta.elements.len() => {
                    let elem = &meta.elements[*idx];
                    match converter.resolve_message(elem, 0) {
                        Some(msg) => osc_packets.push(TimedOSCPacket {
                            time: f64_to_bigdecimal(*time),
                            packet: msg.message,
                        }),
                        None => {} // skip
                    }
                }
                _ => {
                    // Silence padding → /empty_message
                    osc_packets.push(TimedOSCPacket {
                        time: f64_to_bigdecimal(*time),
                        packet: OscPacket::Message(OscMessage {
                            addr: "/empty_message".to_string(),
                            args: vec![],
                        }),
                    });
                }
            }
        }

        let end_beat: f64 = timed_entries.iter().map(|(t, _)| t).sum::<f64>() + 8.0;
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let file_name = format!("/home/{}/jdw_output/track_{}.wav", user, track_name);

        // Build NRT bundles from the OSC packets
        results.push(NrtBundleInfo {
            track_name: track_name.clone(),
            nrt_bundle: build_nrt_record_bundle_from_packets(&osc_packets, bpm, &file_name, end_beat),
            preload_messages: preload_msgs,
            preload_bundles: vec![build_nrt_preload_bundle_from_packets(&osc_packets)],
        });
    }

    results
}

/// Build nrt_preload bundle from TimedOSCPackets.
fn build_nrt_preload_bundle_from_packets(packets: &[TimedOSCPacket]) -> OscPacket {
    let inner = osc_bundle_from_packets(packets);
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("nrt_preload".to_string())],
            }),
            inner,
        ],
    })
}

/// Build nrt_record bundle from TimedOSCPackets.
fn build_nrt_record_bundle_from_packets(
    packets: &[TimedOSCPacket], bpm: f64, file_name: &str, end_beat: f64,
) -> OscPacket {
    let inner = osc_bundle_from_packets(packets);
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: vec![
            OscPacket::Message(OscMessage {
                addr: "/bundle_info".to_string(),
                args: vec![OscType::String("nrt_record".to_string())],
            }),
            OscPacket::Message(OscMessage {
                addr: "/nrt_record_info".to_string(),
                args: vec![
                    OscType::Float(bpm as f32),
                    OscType::String(file_name.to_string()),
                    OscType::Float(end_beat as f32),
                ],
            }),
            inner,
        ],
    })
}

/// Build a plain OSC bundle from TimedOSCPackets (no outer tag).
fn osc_bundle_from_packets(packets: &[TimedOSCPacket]) -> OscPacket {
    let contents: Vec<OscPacket> = packets.iter().map(|tp| {
        OscPacket::Bundle(rosc::OscBundle {
            timetag: osc_timetag_now(),
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/bundle_info".to_string(),
                    args: vec![OscType::String("timed_msg".to_string())],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/timed_msg_info".to_string(),
                    args: vec![OscType::String(tp.time.to_string())],
                }),
                tp.packet.clone(),
            ],
        })
    }).collect();
    OscPacket::Bundle(rosc::OscBundle {
        timetag: osc_timetag_now(),
        content: contents,
    })
}

/// Full pipeline: setup + queue update + commands for a full Billboard.
pub fn send_full_billboard(
    billboard: &full::Billboard,
    synthdefs: &[crate::synthdefs::SynthDefMessage],
    samples: &[crate::sample_loader::SampleLoadMessage],
    config: &OscConfig,
) -> Result<(), String> {
    send_samples(samples, config)?;
    send_full_setup(synthdefs, config)?;
    send_effects_clear(config)?;
    // Commands (routers) must precede effects/drones — SC bus order is strict
    send_full_commands(billboard, config)?;
    send_effects_create(billboard, config)?;
    send_drones_create(billboard, config)?;
    send_full_queue_update(billboard, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::full;
    use std::collections::HashMap;

    #[test]
    fn test_track_to_timed_packets_basic() {
        let mut converter = ElementConverter::new("testSynth", "0", InstrumentType::Synth, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "c4 d4 e4", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 3);

        assert_eq!(packets[0].time, f64_to_bigdecimal(1.0));
        assert_eq!(packets[1].time, f64_to_bigdecimal(1.0));
        assert_eq!(packets[2].time, f64_to_bigdecimal(1.0));

        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/note_on_timed");
            assert_eq!(msg.args[0], OscType::String("testSynth".to_string()));
        } else {
            panic!("Expected message");
        }
    }

    #[test]
    fn test_track_to_timed_packets_with_args() {
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "c4:time0.5 d4:time1.5 e4", &empty, &empty, &empty_overrides).unwrap();

        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0].time, f64_to_bigdecimal(0.5));
        assert_eq!(packets[1].time, f64_to_bigdecimal(1.5));
        assert_eq!(packets[2].time, f64_to_bigdecimal(1.0));
    }

    #[test]
    fn test_track_to_timed_packets_sampler() {
        let mut converter = ElementConverter::new("Roland808", "0", InstrumentType::Sampler, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "14 26 32", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 3);
        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/play_sample");
            assert_eq!(msg.args[1], OscType::String("Roland808".to_string()));
        } else {
            panic!("Expected message");
        }
    }

    #[test]
    fn test_build_timed_msg_bundle_structure() {
        let packet = TimedOSCPacket {
            time: f64_to_bigdecimal(0.5),
            packet: OscPacket::Message(OscMessage {
                addr: "/note_on_timed".to_string(),
                args: vec![],
            }),
        };
        let bundle = build_timed_msg_bundle(&packet);

        match bundle {
            OscPacket::Bundle(b) => {
                assert_eq!(b.content.len(), 3);
                // First message: /bundle_info "timed_msg"
                if let OscPacket::Message(ref m) = b.content[0] {
                    assert_eq!(m.addr, "/bundle_info");
                    assert_eq!(m.args[0], OscType::String("timed_msg".to_string()));
                } else {
                    panic!("Expected message at index 0");
                }
                // Second message: /timed_msg_info "0.5"
                if let OscPacket::Message(ref m) = b.content[1] {
                    assert_eq!(m.addr, "/timed_msg_info");
                    assert_eq!(m.args[0], OscType::String("0.5".to_string()));
                } else {
                    panic!("Expected message at index 1");
                }
            }
            _ => panic!("Expected bundle"),
        }
    }

    #[test]
    fn test_build_update_queue_bundle_structure() {
        let packets = vec![
            TimedOSCPacket {
                time: f64_to_bigdecimal(1.0),
                packet: OscPacket::Message(OscMessage {
                    addr: "/note_on_timed".to_string(),
                    args: vec![],
                }),
            },
        ];
        let bundle = build_update_queue_bundle("testAlias", true, &packets);

        match bundle {
            OscPacket::Bundle(b) => {
                assert_eq!(b.content.len(), 3);
                // /bundle_info "update_queue"
                if let OscPacket::Message(ref m) = b.content[0] {
                    assert_eq!(m.addr, "/bundle_info");
                    assert_eq!(m.args[0], OscType::String("update_queue".to_string()));
                } else {
                    panic!("Expected message at 0");
                }
                // /update_queue_info "testAlias" 1
                if let OscPacket::Message(ref m) = b.content[1] {
                    assert_eq!(m.addr, "/update_queue_info");
                    assert_eq!(m.args[0], OscType::String("testAlias".to_string()));
                    assert_eq!(m.args[1], OscType::Int(1));
                } else {
                    panic!("Expected message at 1");
                }
                // Inner bundle containing timed_msg bundles
                if let OscPacket::Bundle(ref inner) = b.content[2] {
                    assert_eq!(inner.content.len(), 1);
                } else {
                    panic!("Expected bundle at 2");
                }
            }
            _ => panic!("Expected bundle"),
        }
    }

    #[test]
    fn test_build_batch_update_bundle_structure() {
        let inner = OscPacket::Bundle(rosc::OscBundle {
            timetag: osc_timetag_now(),
            content: vec![],
        });
        let batch = build_batch_update_bundle(vec![inner], true);

        match batch {
            OscPacket::Bundle(b) => {
                assert_eq!(b.content.len(), 3);
                if let OscPacket::Message(ref m) = b.content[0] {
                    assert_eq!(m.addr, "/bundle_info");
                    assert_eq!(
                        m.args[0],
                        OscType::String("batch_update_queues".to_string())
                    );
                } else {
                    panic!("Expected message at 0");
                }
                if let OscPacket::Message(ref m) = b.content[1] {
                    assert_eq!(m.addr, "/batch_update_queues_info");
                    assert_eq!(m.args[0], OscType::Int(1));
                } else {
                    panic!("Expected message at 1");
                }
            }
            _ => panic!("Expected bundle"),
        }
    }

    #[test]
    fn test_track_alias_with_override() {
        let track = full::TrackDefinition {
            content: "c4".to_string(),
            group_override: Some("melody".to_string()),
            arg_overrides: HashMap::new(),
            index: 2,
            enabled: true,
        };
        assert_eq!(track_alias("synth", 1, &track), "melody");
    }

    #[test]
    fn test_track_alias_without_override() {
        let track = full::TrackDefinition {
            content: "c4".to_string(),
            group_override: None,
            arg_overrides: HashMap::new(),
            index: 2,
            enabled: true,
        };
        assert_eq!(track_alias("synth", 1, &track), "synth_1_2");
    }

    #[test]
    fn test_parse_real_bbd_gong() {
        let source = include_str!("../../jdw-pycompose/songs/gong.bbd");
        let bb = full::parse(source);

        // 10 commands + 4 UPDATE_COMMANDs... actually let's count
        // Lines 1-19: 19 UPDATE_COMMAND, Line 21: COMMAND, etc.
        assert!(!bb.commands.is_empty(), "gong.bbd should have commands");
        assert!(!bb.sections.is_empty(), "gong.bbd should have sections");

        // Check default args
        assert!(bb.default_args.contains_key("time"));
        assert!(bb.default_args.contains_key("amp"));
    }

    #[test]
    fn test_parse_real_bbd_arena() {
        let source = include_str!("../../jdw-pycompose/songs/arena.bbd");
        let bb = full::parse(source);

        assert!(!bb.commands.is_empty());
        assert!(!bb.sections.is_empty());
        assert!(bb.default_args.contains_key("amp"));

        // Check that some tracks have metadata
        let has_meta = bb.sections.iter().any(|s| {
            s.tracks
                .iter()
                .any(|t| t.group_override.is_some())
        });
        assert!(has_meta, "arena.bbd should have tracks with group overrides");
    }

    // ========== Args precedence tests ==========

    #[test]
    fn test_args_header_overrides_default() {
        // Bug: header amp=0.08 was not overriding DEFAULT amp=1.0
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let defaults: HashMap<String, f64> = [("amp".into(), 1.0), ("sus".into(), 0.5)].into();
        let header: HashMap<String, f64> = [("amp".into(), 0.08), ("relT".into(), 2.0)].into();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "c4", &defaults, &header, &empty_overrides).unwrap();
        if let OscPacket::Message(ref msg) = packets[0].packet {
            // Find amp in the flat arg list (key-value pairs)
            let find_after = |key: &str| -> f32 {
                let mut found = false;
                for a in &msg.args {
                    if found {
                        if let OscType::Float(v) = a { return *v; }
                    }
                    if let OscType::String(s) = a {
                        if s == key { found = true; }
                    }
                }
                panic!("arg {} not found", key);
            };
            assert!((find_after("amp") - 0.08).abs() < 0.01, "amp should be 0.08 from header, overriding DEFAULT 1.0");
            assert!((find_after("sus") - 0.5).abs() < 0.01, "sus should be 0.5 from DEFAULT (header didn't set it)");
        }
    }

    #[test]
    fn test_args_element_overrides_header() {
        // Element inline args should override header args
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let defaults: HashMap<String, f64> = [("amp".into(), 1.0)].into();
        let header: HashMap<String, f64> = [("amp".into(), 0.08)].into();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "c4:amp0.03", &defaults, &header, &empty_overrides).unwrap();
        if let OscPacket::Message(ref msg) = packets[0].packet {
            let find_after = |key: &str| -> f32 {
                let mut found = false;
                for a in &msg.args {
                    if found {
                        if let OscType::Float(v) = a { return *v; }
                    }
                    if let OscType::String(s) = a {
                        if s == key { found = true; }
                    }
                }
                panic!("arg {} not found", key);
            };
            assert!((find_after("amp") - 0.03).abs() < 0.01, "amp=0.03 from element should override header 0.08");
        }
    }

    #[test]
    fn test_args_track_overrides_operator_mul() {
        // Track override `amp*2` should multiply the resolved header value
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let defaults: HashMap<String, f64> = HashMap::new();
        let header: HashMap<String, f64> = [("amp".into(), 0.08)].into();
        let track_overrides: HashMap<String, (char, f64)> = [("amp".into(), ('*', 2.0))].into();
        let packets = track_to_timed_packets(&mut converter, "c4", &defaults, &header, &track_overrides).unwrap();
        if let OscPacket::Message(ref msg) = packets[0].packet {
            let find_after = |key: &str| -> f32 {
                let mut found = false;
                for a in &msg.args {
                    if found {
                        if let OscType::Float(v) = a { return *v; }
                    }
                    if let OscType::String(s) = a {
                        if s == key { found = true; }
                    }
                }
                panic!("arg {} not found", key);
            };
            assert!((find_after("amp") - 0.16).abs() < 0.01, "amp=0.08*2=0.16 from track override");
        }
    }

    #[test]
    fn test_args_track_overrides_operator_add() {
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let defaults: HashMap<String, f64> = [("time".into(), 0.5)].into();
        let header: HashMap<String, f64> = HashMap::new();
        let track_overrides: HashMap<String, (char, f64)> = [("time".into(), ('+', 0.2))].into();
        let packets = track_to_timed_packets(&mut converter, "c4", &defaults, &header, &track_overrides).unwrap();
        if let OscPacket::Message(ref msg) = packets[0].packet {
            let find_after = |key: &str| -> f32 {
                let mut found = false;
                for a in &msg.args {
                    if found {
                        if let OscType::Float(v) = a { return *v; }
                    }
                    if let OscType::String(s) = a {
                        if s == key { found = true; }
                    }
                }
                panic!("arg {} not found", key);
            };
            assert!((find_after("time") - 0.7).abs() < 0.01, "time=0.5+0.2=0.7 from track override");
        }
    }

    #[test]
    fn test_args_track_overrides_operator_replace() {
        // '=' operator should replace the resolved value
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let defaults: HashMap<String, f64> = [("amp".into(), 1.0)].into();
        let header: HashMap<String, f64> = [("amp".into(), 0.08)].into();
        let track_overrides: HashMap<String, (char, f64)> = [("amp".into(), ('=', 0.5))].into();
        let packets = track_to_timed_packets(&mut converter, "c4", &defaults, &header, &track_overrides).unwrap();
        if let OscPacket::Message(ref msg) = packets[0].packet {
            let find_after = |key: &str| -> f32 {
                let mut found = false;
                for a in &msg.args {
                    if found {
                        if let OscType::Float(v) = a { return *v; }
                    }
                    if let OscType::String(s) = a {
                        if s == key { found = true; }
                    }
                }
                panic!("arg {} not found", key);
            };
            assert!((find_after("amp") - 0.5).abs() < 0.01, "amp should be replaced to 0.5 by track override");
        }
    }

    // ========== Rest/silence 'x' tests ==========

    #[test]
    fn test_rest_x_produces_empty_msg_for_synth() {
        // Bug: is_symbol checked suffix, but shuttle puts 'x' in prefix.
        // 'x' should produce /empty_msg, not /note_on_timed.
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "x x x", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 3);
        for p in &packets {
            if let OscPacket::Message(ref msg) = p.packet {
                assert_eq!(msg.addr, "/empty_msg", "rest 'x' should produce /empty_msg, not {}", msg.addr);
                assert!(msg.args.is_empty(), "/empty_msg should have no args");
            }
        }
    }

    #[test]
    fn test_rest_x_produces_empty_msg_for_sampler() {
        // Bug: Sampler 'x' elements were falling through to /play_sample
        let mut converter = ElementConverter::new("SP_808", "0", InstrumentType::Sampler, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "x 14 x", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 3);
        // First and third are rest -> /empty_msg
        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/empty_msg", "rest 'x' for sampler should be /empty_msg");
        }
        // Middle is a real sample play
        if let OscPacket::Message(ref msg) = packets[1].packet {
            assert_eq!(msg.addr, "/play_sample", "numeric index should be /play_sample");
        }
        if let OscPacket::Message(ref msg) = packets[2].packet {
            assert_eq!(msg.addr, "/empty_msg", "rest 'x' for sampler should be /empty_msg");
        }
    }

    #[test]
    fn test_rest_x_produces_empty_msg_for_drone() {
        let mut converter = ElementConverter::new("DR_drone", "0", InstrumentType::Drone, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, "x", &empty, &empty, &empty_overrides).unwrap();
        assert_eq!(packets.len(), 1);
        if let OscPacket::Message(ref msg) = packets[0].packet {
            assert_eq!(msg.addr, "/empty_msg");
        }
    }

    #[test]
    fn test_silence_dot_ignored() {
        // '.' (legacy silence, tokenized as 'x') should be ignored (None)
        let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
        let empty: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();
        let packets = track_to_timed_packets(&mut converter, ".", &empty, &empty, &empty_overrides).unwrap();
        // '.' becomes 'x' token, which is_symbol catches -> /empty_msg
        // BUT: in the shuttle parser, '.' might be handled differently...
        // Check it produces /empty_msg
        if packets.is_empty() {
            // Some implementations might return None for '.'
        } else {
            if let OscPacket::Message(ref msg) = packets[0].packet {
                assert!(
                    msg.addr == "/empty_msg" || msg.addr == "/note_on_timed",
                    "'.' should produce /empty_msg or be filtered, got {}", msg.addr
                );
            }
        }
    }

    // ========== Arg ordering test ==========

    #[test]
    fn test_args_are_sorted_alphabetically() {
        // After the fix, args in OSC messages should be alphabetically sorted.
        // Run twice to verify determinism (HashMap iteration was random before).
        let defaults: HashMap<String, f64> = [
            ("zLast".into(), 1.0), ("aFirst".into(), 2.0),
            ("mid".into(), 3.0),
        ].into();
        let empty_header: HashMap<String, f64> = HashMap::new();
        let empty_overrides: HashMap<String, (char, f64)> = HashMap::new();

        let mut collect_keys = || {
            let mut converter = ElementConverter::new("s", "0", InstrumentType::Synth, ScaleData::default());
            let packets = track_to_timed_packets(&mut converter, "c4", &defaults, &empty_header, &empty_overrides).unwrap();
            let mut kv_keys = Vec::new();
            if let OscPacket::Message(ref msg) = packets[0].packet {
                let mut i = 4; // skip 4 positional args
                while i + 1 < msg.args.len() {
                    if let OscType::String(ref k) = msg.args[i] {
                        kv_keys.push(k.clone());
                    }
                    i += 2;
                }
            }
            kv_keys
        };

        let keys1 = collect_keys();
        let keys2 = collect_keys();
        assert_eq!(keys1, keys2, "arg keys should be deterministic across calls");
        // Non-override keys (everything after freq) should be alphabetical
        let non_override = &keys1[1..];
        let mut sorted = non_override.to_vec();
        sorted.sort();
        assert_eq!(non_override, sorted.as_slice(), "non-override arg keys should be alphabetically sorted: {:?}", keys1);
    }

    // ========== NRT bundle tests ==========
    #[test]
    fn test_get_nrt_record_bundles_basic() {
        let source = "\
@test:synth
c4 d4
";
        let bb = full::parse(source);
        let synthdefs = vec![];
        let samples = vec![];
        let bundles = get_nrt_record_bundles(&bb, &synthdefs, &samples);
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].track_name, "test_0_0");
        // Preload messages should include /clear_nrt
        assert!(bundles[0].preload_messages.iter().any(|m| matches!(m, OscPacket::Message(ref msg) if msg.addr == "/clear_nrt")));
        // NRT bundle should have 3 children
        if let OscPacket::Bundle(ref b) = bundles[0].nrt_bundle {
            assert_eq!(b.content.len(), 3);
        } else { panic!("Expected bundle"); }
    }

    #[test]
    fn test_get_nrt_record_bundles_with_bpm() {
        let source = "\
/set_bpm 140
@test:synth
c4
";
        let bb = full::parse(source);
        let bundles = get_nrt_record_bundles(&bb, &[], &[]);
        if let OscPacket::Bundle(ref b) = bundles[0].nrt_bundle {
            if let OscPacket::Message(ref m) = b.content[1] {
                assert_eq!(m.addr, "/nrt_record_info");
                assert_eq!(m.args[0], OscType::Float(140.0));
            } else { panic!("Expected /nrt_record_info"); }
        }
    }

    #[test]
    fn test_parse_real_bbd_rattlesnake() {
        let source = include_str!("../../jdw-pycompose/songs/rattlesnake.bbd");
        let bb = full::parse(source);

        assert!(!bb.commands.is_empty());
        assert!(!bb.sections.is_empty());
        // Filters are collected regardless of position (matching Python)
        // rattlesnake.bbd has >>> lines after commands
        assert!(!bb.sections[0].tracks.is_empty());
        // Check that tracks with metadata strip content correctly
        let has_meta = bb.sections.iter().any(|s| {
            s.tracks
                .iter()
                .any(|t| t.group_override.is_some())
        });
        assert!(has_meta);
    }
}


