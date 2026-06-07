use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use rosc::encoder;
use rosc::{OscMessage, OscPacket, OscType};

use crate::billboard::{Billboard, Track};

const ROUTER_DEFAULT: &str = "127.0.0.1:13339";

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

/// A timed OSC bundle targeted at the sequencer.
pub struct SequencerBundle {
    pub beat: f64,
    pub packet: OscPacket,
}

/// Send a `/hard_stop` message to the router for the sequencer.
pub fn send_stop(config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;
    sock.set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| format!("set timeout: {}", e))?;

    let msg = OscPacket::Message(OscMessage {
        addr: "/hard_stop".to_string(),
        args: vec![],
    });
    send_to_router(&sock, &config.router_addr, &msg)?;

    // Also send wipe_on_finish
    let wipe = OscPacket::Message(OscMessage {
        addr: "/wipe_on_finish".to_string(),
        args: vec![],
    });
    send_to_router(&sock, &config.router_addr, &wipe)?;

    Ok(())
}

/// Generate and send sequencer queue update messages for a billboard.
pub fn send_queue_update(billboard: &Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    for track in &billboard.tracks {
        if !track.enabled {
            continue;
        }
        let bundles = track_to_sequencer_bundles(track, config);
        let packet = build_batch_update_packet(track, &bundles, config)?;
        send_to_router(&sock, &config.router_addr, &packet)?;
    }

    Ok(())
}

/// Generate and send setup messages (synthdef loading, sample loading).
pub fn send_setup(billboard: &Billboard, config: &OscConfig) -> Result<(), String> {
    let sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| format!("bind: {}", e))?;

    // Collect unique synth names and send /read_scd for each
    let mut synths_seen = std::collections::HashSet::new();
    for track in &billboard.tracks {
        if !track.enabled {
            continue;
        }
        if synths_seen.insert(track.synth.clone()) {
            let msg = OscPacket::Message(OscMessage {
                addr: "/read_scd".to_string(),
                args: vec![OscType::String(format!(
                    "~jdw.synth(\"{}\").add;",
                    track.synth
                ))],
            });
            send_to_router(&sock, &config.router_addr, &msg)?;
        }
    }

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

// -- Internal helpers --

fn send_to_router(sock: &UdpSocket, addr: &str, packet: &OscPacket) -> Result<(), String> {
    let target: SocketAddr = addr.parse().map_err(|e| format!("invalid addr: {}", e))?;
    let buf = encoder::encode(packet).map_err(|e| format!("encode: {}", e))?;
    sock.send_to(&buf, target).map_err(|e| format!("send: {}", e))?;
    Ok(())
}

fn track_to_sequencer_bundles(track: &Track, config: &OscConfig) -> Vec<SequencerBundle> {
    let mut bundles = Vec::new();
    let mut beat = 0.0;

    for elem in &track.elements {
        let note_len = elem.args.get("time").copied().unwrap_or(1.0);
        let gate = elem.args.get("sus").copied().unwrap_or(note_len * 0.8);

        let msg = OscMessage {
            addr: "/note_on_timed".to_string(),
            args: vec![
                OscType::String(track.synth.clone()),
                OscType::String(format!(
                    "{}_{}_{}_{{nodeId}}",
                    track.name, track.synth, config.external_id_counter
                )),
                OscType::Float(gate as f32),
                OscType::Float(0.0), // delay ms
                OscType::String(format!("{}", elem.prefix)),
            ],
        };

        bundles.push(SequencerBundle {
            beat,
            packet: OscPacket::Message(msg),
        });

        beat += note_len;
    }

    bundles
}

fn build_batch_update_packet(
    _track: &Track,
    bundles: &[SequencerBundle],
    _config: &OscConfig,
) -> Result<OscPacket, String> {
    use rosc::{OscBundle, OscTime};
    use std::convert::TryFrom;

    let mut messages = Vec::new();

    // Header: /bundle_info
    messages.push(OscPacket::Message(OscMessage {
        addr: "/bundle_info".to_string(),
        args: vec![OscType::String("batch_update_queues".to_string())],
    }));

    for bundle in bundles {
        let time_f = bundle.beat * 0.5; // convert beats to seconds for OscTime
        let duration = Duration::from_secs_f64(time_f);
        let system_time = std::time::SystemTime::UNIX_EPOCH + duration;

        let timed_packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime::try_from(system_time).map_err(|e| format!("time: {}", e))?,
            content: vec![bundle.packet.clone()],
        });

        messages.push(timed_packet);
    }

    Ok(OscPacket::Bundle(OscBundle {
        timetag: OscTime::try_from(std::time::SystemTime::now()).map_err(|e| format!("time: {}", e))?,
        content: messages,
    }))
}
