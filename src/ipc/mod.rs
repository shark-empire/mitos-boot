//! IPC between mitos-boot and mitos-init (§23).
//!
//! Security model:
//!   - Unix domain socket at a configured /run path, mode 0600.
//!   - Every accepted connection is checked with SO_PEERCRED; the peer uid
//!     must equal our euid. Anything else is dropped without a reply.
//!   - Length-prefixed binary frames with a hard payload cap; nothing is
//!     allocated before lengths are validated.
//!   - At most MAX_CLIENTS concurrent connections.
//!   - The socket file is only ever unlinked when it is actually a socket.

pub mod init;
pub use init::Message;

use crate::config::Config;
use crate::error::BootError;
use crate::state::SharedState;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_CLIENTS: usize = 4;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const ACCEPT_POLL: Duration = Duration::from_millis(50);

struct Client { id: u64, stream: UnixStream }

pub struct IpcServer {
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    clients: Arc<Mutex<Vec<Client>>>,
    state: SharedState,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    next_id: Arc<AtomicU64>,
}

impl IpcServer {
    pub fn bind(cfg: &Config, state: SharedState) -> Result<Self, BootError> {
        let path = PathBuf::from(&cfg.ipc.socket);
        let parent = path.parent()
            .ok_or_else(|| BootError::Ipc("socket path has no parent".into()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| BootError::Ipc(format!("mkdir {}: {e}", parent.display())))?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755));

        // Only ever remove a stale *socket*, never a regular file.
        match std::fs::symlink_metadata(&path) {
            Ok(md) if md.file_type().is_socket() => { let _ = std::fs::remove_file(&path); }
            Ok(_) => {
                return Err(BootError::Ipc(format!(
                    "{} exists and is not a socket", path.display())));
            }
            Err(_) => {}
        }

        let listener = UnixListener::bind(&path)
            .map_err(|e| BootError::Ipc(format!("bind: {e}")))?;
        // Root-only socket, regardless of umask.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| BootError::Ipc(format!("chmod: {e}")))?;

        log::debug!("ipc: listening on {}", path.display());
        Ok(IpcServer {
            socket_path: path,
            listener: Some(listener),
            clients: Arc::new(Mutex::new(Vec::new())),
            state,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn start(&mut self) {
        let Some(listener) = self.listener.take() else { return };
        let _ = listener.set_nonblocking(true);
        let stop = Arc::clone(&self.stop);
        let clients = Arc::clone(&self.clients);
        let state = self.state.clone();
        let next_id = Arc::clone(&self.next_id);

        self.thread = Some(std::thread::spawn(move || loop {
            if stop.load(Ordering::Relaxed) { break; }
            match listener.accept() {
                Ok((stream, _)) => {
                    if stop.load(Ordering::Relaxed) { break; }
                    let count = clients.lock().map(|c| c.len()).unwrap_or(MAX_CLIENTS);
                    if count >= MAX_CLIENTS {
                        log::warn!("ipc: too many clients; refusing connection");
                        continue;
                    }
                    if !peer_allowed(&stream) {
                        log::warn!("ipc: rejected connection from wrong uid");
                        continue;
                    }
                    let id = next_id.fetch_add(1, Ordering::Relaxed);
                    let Ok(write_half) = stream.try_clone() else { continue };
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                    let _ = write_half.set_write_timeout(Some(IO_TIMEOUT));
                    if let Ok(mut c) = clients.lock() { c.push(Client { id, stream: write_half }); }

                    let clients_t = Arc::clone(&clients);
                    let state_t = state.clone();
                    std::thread::spawn(move || serve_client(id, stream, clients_t, state_t));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(e) => {
                    log::debug!("ipc accept: {e}");
                    std::thread::sleep(ACCEPT_POLL);
                }
            }
        }));
    }

    /// Broadcast an event to every connected client. Dead clients are pruned.
    /// Never blocks the boot longer than the per-client write timeout.
    pub fn send(&self, m: &Message) {
        let payload = m.to_bytes();
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain_mut(|c| {
                c.stream.write_all(&frame).and_then(|_| c.stream.flush()).is_ok()
            });
        }
    }

    /// Explicit shutdown; the same cleanup runs in Drop.
    pub fn shutdown(self) { /* Drop performs cleanup */ }

    fn cleanup(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() { let _ = h.join(); }
        if let Ok(mut c) = self.clients.lock() { c.clear(); }
        if let Ok(md) = std::fs::symlink_metadata(&self.socket_path) {
            if md.file_type().is_socket() {
                let _ = std::fs::remove_file(&self.socket_path);
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) { self.cleanup(); }
}

fn serve_client(id: u64, mut stream: UnixStream, clients: Arc<Mutex<Vec<Client>>>, state: SharedState) {
    loop {
        match read_frame(&mut stream) {
            Ok(Some(bytes)) => {
                let reply = match Message::decode(&bytes) {
                    Ok(m) => dispatch(m, &state),
                    Err(e) => Message::Error(e.to_string()),
                };
                if write_frame(&mut stream, &reply).is_err() { break; }
            }
            Ok(None) => break, // clean EOF
            Err(_) => break,   // malformed / timed out: drop the client
        }
    }
    if let Ok(mut c) = clients.lock() { c.retain(|cl| cl.id != id); }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn dispatch(m: Message, state: &SharedState) -> Message {
    match m {
        Message::InitReady => {
            state.set_init_ready(true);
            log::info!("ipc: mitos-init reports ready");
            Message::Ack
        }
        Message::SetSystemReady => { state.set_system_ready(true); Message::Ack }
        Message::Ping => Message::Ack,
        other => Message::Error(format!("unexpected message {other:?}")),
    }
}

fn read_frame(stream: &mut UnixStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match stream.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > init::MAX_LEN {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData, "bad frame length"));
    }
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn write_frame(stream: &mut UnixStream, m: &Message) -> std::io::Result<()> {
    let payload = m.to_bytes();
    stream.write_all(&(payload.len() as u32).to_le_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()
}

/// SO_PEERCRED: the peer must share our euid. Prevents any other local user
/// from talking to the boot socket even if permissions were ever wrong.
fn peer_allowed(stream: &UnixStream) -> bool {
    // SAFETY: cred is a valid out-pointer of the size passed in len.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void, &mut len,
        ) == 0
    };
    ok && len as usize >= std::mem::size_of::<libc::ucred>()
        && cred.uid == unsafe { libc::geteuid() }
}