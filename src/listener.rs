/// OSC Listener — blocks waiting for NRT completion responses.
///
/// Spawns a background UDP OSC server that records incoming message addresses.
/// `wait_for()` blocks until the expected address arrives, with a timeout.
///
/// Port of Python's `Listener` class from jdw-pycompose/listener.py.
use std::net::UdpSocket;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Default timeout in seconds for NRT response.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// An OSC listener that records incoming message addresses.
pub struct Listener {
    /// Shared list of received addresses (addr, args preview string).
    received: Arc<Mutex<Vec<(String, String)>>>,
    /// Handle to stop the listener.
    stop_tx: Option<mpsc::Sender<()>>,
    /// Join handle for the background thread.
    thread: Option<thread::JoinHandle<()>>,
}

impl Listener {
    /// Start listening on `port`, returns the bound listener.
    ///
    /// If `port` is 0, the OS assigns a random available port.
    pub fn start(port: u16) -> Result<Self, String> {
        let addr = format!("127.0.0.1:{}", port);
        let socket = UdpSocket::bind(&addr).map_err(|e| format!("Listener bind: {}", e))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| format!("set_read_timeout: {}", e))?;

        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();

        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let handle = thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                // Check for stop signal
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                match socket.recv_from(&mut buf) {
                    Ok((len, _src)) => {
                        // Parse the OSC packet
                        let addr = parse_osc_address(&buf[..len]);
                        if !addr.is_empty() {
                            let preview = extract_first_arg(&buf[..len]);
                            if let Ok(mut r) = received_clone.lock() {
                                r.push((addr, preview));
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Timeout — check for stop signal next iteration
                        continue;
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Listener {
            received,
            stop_tx: Some(stop_tx),
            thread: Some(handle),
        })
    }

    /// Block until `addr` appears in received messages, with timeout.
    ///
    /// Returns `true` if the address was received, `false` on timeout.
    pub fn wait_for(&self, addr: &str, timeout_secs: u64) -> bool {
        let start = Instant::now();
        let timeout = Duration::from_secs(timeout_secs);
        let before = self.received.lock().map(|r| r.len()).unwrap_or(0);

        loop {
            if start.elapsed() >= timeout {
                return false;
            }
            let count = self.received.lock().map(|r| r.len()).unwrap_or(0);
            if count > before {
                // Check if our address appeared
                if let Ok(r) = self.received.lock() {
                    if r.iter().any(|(a, _)| a == addr) {
                        return true;
                    }
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Block until `/nrt_record_finished` is received.
    pub fn wait_for_nrt(&self) -> bool {
        self.wait_for("/nrt_record_finished", DEFAULT_TIMEOUT_SECS)
    }

    /// Get the first received response value (for /nrt_record_finished: status + filename).
    pub fn get_response(&self) -> Option<(String, String)> {
        self.received
            .lock()
            .ok()
            .and_then(|r| {
                r.iter()
                    .find(|(a, _)| a == "/nrt_record_finished")
                    .map(|(_, preview)| {
                        let parts: Vec<&str> = preview.splitn(2, '|').collect();
                        (parts[0].to_string(), parts.get(1).unwrap_or(&"").to_string())
                    })
            })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Parse the address from an OSC packet buffer (first null-terminated string).
fn parse_osc_address(buf: &[u8]) -> String {
    if buf.is_empty() || buf[0] != b'/' {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

/// Extract the first string arg from an OSC packet for preview.
fn extract_first_arg(buf: &[u8]) -> String {
    // Skip address
    let addr_end = buf.iter().position(|&b| b == 0).unwrap_or(0);
    let mut off = addr_end + 1;
    while off < buf.len() && off % 4 != 0 { off += 1; } // pad to 4
    // Skip type tag
    if off < buf.len() && buf[off] == b',' {
        let tag_end = buf[off..].iter().position(|&b| b == 0).unwrap_or(0);
        off += tag_end + 1;
        while off < buf.len() && off % 4 != 0 { off += 1; }
    }
    // Read first string arg
    if off < buf.len() {
        let val_end = buf[off..].iter().position(|&b| b == 0).unwrap_or(buf.len() - off);
        return String::from_utf8_lossy(&buf[off..off + val_end]).to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_listener_start_stop() {
        let listener = Listener::start(0).expect("listener should start");
        // Should bind to a random port and be droppable without panic
        drop(listener);
    }

    #[test]
    fn test_listener_wait_for_timeout() {
        let listener = Listener::start(0).unwrap();
        let result = listener.wait_for("/nrt_record_finished", 1);
        assert!(!result, "should timeout when no message sent");
    }

    #[test]
    fn test_listener_wait_for_receives() {
        let listener = Listener::start(0).unwrap();
        // Send a message to the listener's port
        let _actual_port = {
            // Hack: we need to know the port. Since we can't easily get it from the listener,
            // just test with a known port.
            // Let's use 13500 for test.
            drop(listener);
            let l = Listener::start(13500).unwrap();
            // Send an OSC message
            let addr = b"/nrt_record_finished\0\0\0,s\0\0SUCCESS\0";
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.send_to(addr, "127.0.0.1:13500").unwrap();
            let result = l.wait_for("/nrt_record_finished", 3);
            assert!(result, "should receive the message");
            drop(l);
        };
    }
}
