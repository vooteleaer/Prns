//! RNode over Bluetooth LE on Android. btleplug cannot reach Android's GATT stack from native
//! code, so `ble://` RNode interfaces open through the app instead: Rust queues a connect request,
//! `RNodeBleLink.kt` finds the RNode, connects, and subscribes to its Nordic UART service, then the
//! two sides exchange the raw byte stream through these JNI entry points.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use personal_rns::config::RNodeBleTarget;
use personal_rns::rnode::{install_platform_ble_opener, RNodeBleOpener};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::{mpsc, oneshot};

const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_BUFFER: usize = 32_768;
const OUTBOUND_CHUNK: usize = 512;

/// One request the app has not picked up yet: which RNode to connect to.
pub(crate) struct ConnectRequest {
    pub(crate) session: u32,
    /// `""` for the first bonded RNode, `"addr:<MAC>"`, or `"name:<device name>"`.
    pub(crate) target: String,
}

struct Session {
    opened: Option<oneshot::Sender<Result<(), String>>>,
    inbound: mpsc::UnboundedSender<Vec<u8>>,
    outbound: VecDeque<Vec<u8>>,
    closed: bool,
}

#[derive(Default)]
struct Bridge {
    requests: VecDeque<ConnectRequest>,
    sessions: HashMap<u32, Session>,
}

struct Shared {
    bridge: Mutex<Bridge>,
    work: Condvar,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static NEXT_SESSION: AtomicU32 = AtomicU32::new(1);

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| Shared {
        bridge: Mutex::new(Bridge::default()),
        work: Condvar::new(),
    })
}

fn lock() -> MutexGuard<'static, Bridge> {
    shared()
        .bridge
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Route `ble://` RNodes through the app. Safe to call on every engine start.
pub(crate) fn install() {
    let opener: RNodeBleOpener = Arc::new(|target| Box::pin(open(target)));
    let _ = install_platform_ble_opener(opener);
}

fn encode_target(target: &RNodeBleTarget) -> String {
    match target {
        RNodeBleTarget::FirstBondedRnode => String::new(),
        RNodeBleTarget::Address(address) => format!("addr:{address}"),
        RNodeBleTarget::Name(name) => format!("name:{}", name.as_str()),
    }
}

async fn open(target: RNodeBleTarget) -> io::Result<DuplexStream> {
    let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let (opened_tx, opened_rx) = oneshot::channel();
    let (inbound_tx, mut inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    {
        let mut bridge = lock();
        bridge.sessions.insert(
            session,
            Session {
                opened: Some(opened_tx),
                inbound: inbound_tx,
                outbound: VecDeque::new(),
                closed: false,
            },
        );
        bridge.requests.push_back(ConnectRequest {
            session,
            target: encode_target(&target),
        });
    }
    shared().work.notify_all();
    log::info!("hopspot: RNode BLE session {session} requested for {target}");

    let opened = tokio::time::timeout(OPEN_TIMEOUT, opened_rx).await;
    let failure = match opened {
        Ok(Ok(Ok(()))) => None,
        Ok(Ok(Err(reason))) => Some(reason),
        Ok(Err(_)) => Some("the app dropped the Bluetooth request".to_string()),
        Err(_) => Some(format!(
            "no RNode connection within {} seconds; pair the RNode in Android Bluetooth settings and keep it in range",
            OPEN_TIMEOUT.as_secs()
        )),
    };
    if let Some(reason) = failure {
        close(session);
        return Err(io::Error::other(reason));
    }

    let (driver, mut bridge_end) = tokio::io::duplex(STREAM_BUFFER);
    tokio::spawn(async move {
        let mut buffer = vec![0u8; OUTBOUND_CHUNK];
        loop {
            tokio::select! {
                read = bridge_end.read(&mut buffer) => match read {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        {
                            let mut bridge = lock();
                            let Some(entry) = bridge.sessions.get_mut(&session) else { break };
                            entry.outbound.push_back(buffer[..read].to_vec());
                        }
                        shared().work.notify_all();
                    }
                },
                inbound = inbound_rx.recv() => match inbound {
                    Some(bytes) => {
                        if bridge_end.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        close(session);
        log::info!("hopspot: RNode BLE session {session} closed");
    });
    Ok(driver)
}

fn close(session: u32) {
    if let Some(entry) = lock().sessions.get_mut(&session) {
        entry.closed = true;
    }
    shared().work.notify_all();
}

/// Wait up to `timeout` for a connect request.
pub(crate) fn next_request(timeout: Duration) -> Option<ConnectRequest> {
    let shared = shared();
    let bridge = lock();
    let (mut bridge, _) = shared
        .work
        .wait_timeout_while(bridge, timeout, |bridge| bridge.requests.is_empty())
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    bridge.requests.pop_front()
}

/// The app reports whether the GATT link is ready to carry bytes.
pub(crate) fn opened(session: u32, result: Result<(), String>) {
    let failed = result.is_err();
    let sender = lock()
        .sessions
        .get_mut(&session)
        .and_then(|entry| entry.opened.take());
    if let Some(sender) = sender {
        let _ = sender.send(result);
    }
    if failed {
        lock().sessions.remove(&session);
    }
}

/// Bytes the RNode notified on its TX characteristic.
pub(crate) fn inbound(session: u32, bytes: Vec<u8>) -> bool {
    lock()
        .sessions
        .get(&session)
        .is_some_and(|entry| entry.inbound.send(bytes).is_ok())
}

pub(crate) enum Outbound {
    Bytes(Vec<u8>),
    Idle,
    Closed,
}

/// Wait up to `timeout` for bytes to write to the RNode's RX characteristic.
pub(crate) fn take_outbound(session: u32, timeout: Duration) -> Outbound {
    let shared = shared();
    let bridge = lock();
    let (mut bridge, _) = shared
        .work
        .wait_timeout_while(bridge, timeout, |bridge| {
            bridge
                .sessions
                .get(&session)
                .is_some_and(|entry| entry.outbound.is_empty() && !entry.closed)
        })
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = bridge.sessions.get_mut(&session) else {
        return Outbound::Closed;
    };
    if let Some(bytes) = entry.outbound.pop_front() {
        return Outbound::Bytes(bytes);
    }
    if entry.closed {
        bridge.sessions.remove(&session);
        return Outbound::Closed;
    }
    Outbound::Idle
}

/// The GATT link dropped; the RNode interface sees end-of-stream and reconnects.
pub(crate) fn disconnected(session: u32) {
    let removed = lock().sessions.remove(&session);
    if let Some(mut entry) = removed {
        if let Some(sender) = entry.opened.take() {
            let _ = sender.send(Err("the RNode disconnected".to_string()));
        }
    }
    shared().work.notify_all();
}
