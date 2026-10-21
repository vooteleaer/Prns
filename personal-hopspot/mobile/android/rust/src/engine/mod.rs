mod configured;
mod persistence;
mod sideband_join;
mod worker;

use core::fmt::Write as _;
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use personal_hopspot_core::{
    card_label, CardKind, CardLabel, MobileDiscoveryGroupOutcome, MobileDiscoveryInterface,
    MobileEngineFailure, MobileEngineState,
};
use personal_rns::bluetooth_auto::BluetoothAutoStatus;
use personal_rns::engine::{AnnounceAppData, AnnounceNow, AnnounceTarget, PrnsCommand};
use personal_rns::identity::IdentityHash;
use personal_rns::interfaces::bluetooth_auto::BleIdentity;
use personal_rns::interfaces::{
    ConnectionState, DiscoveryGroupApplyOutcome, DiscoveryGroupSet, InterfaceId, InterfaceKind,
    InterfaceSnapshot, InterfaceStatus, Membership,
};
use personal_rns::manifold::tokio::TokioInterfaceStatus;
use personal_rns::runtime::{PrnsNodeHandle, RuntimeHealth};
use personal_rns::shared_instance::rns_rpc::RpcAuthenticationKey;
#[cfg(unix)]
use personal_rns::shared_instance::SharedInstanceClient;
use personal_rns::wifi_auto::AutoWifiStatus;
use personal_rns::wifi_aware::WifiAwareStatus;
use personal_rns::wifi_direct::WifiDirectStatus;
use personal_rns::wire::DestinationHash;
use tokio::sync::oneshot;

use crate::bluetooth_auto::AndroidBleBridge;
use crate::bridge::AndroidUsbBridge;
use crate::service_discovery::AndroidServiceDiscoveryBridge;
use crate::wifi_aware::AndroidWifiAwareBridge;
use crate::wifi_direct::AndroidWifiDirectBridge;
use configured::ConfiguredInterfaces;
use persistence::{PersistenceHealth, PersistenceSnapshot};
use personal_rns::PlanRuntimeContext;

const USB_INTERFACE_ID: InterfaceId = InterfaceId::new([0xD0; 8]);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) const ANDROID_PORT: &str = "android-usb";
pub(super) const LOCAL_RNS_PORT: u16 = 37428;
pub(super) const RPC_PORT: u16 = LOCAL_RNS_PORT + 1;
pub(super) const ANNOUNCE_APP_DATA: &[u8] = b"personal-hopspot";
pub(super) const NODE_ANNOUNCE_APP_DATA: &[u8] = b"personal-hopspot";
pub(super) const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy)]
pub(super) struct EnginePorts {
    pub(super) local: u16,
    pub(super) rpc: u16,
}

impl EnginePorts {
    const ANDROID: Self = Self {
        local: LOCAL_RNS_PORT,
        rpc: RPC_PORT,
    };

    #[cfg(test)]
    const EPHEMERAL: Self = Self { local: 0, rpc: 0 };
}

pub(super) struct EngineResources {
    pub(super) started_at: Instant,
    pub(super) node_identity_hash: IdentityHash,
    pub(super) ble_identity: BleIdentity,
    pub(super) usb_status: TokioInterfaceStatus,
    pub(super) wifi_status: AutoWifiStatus,
    pub(super) ble_status: BluetoothAutoStatus,
    pub(super) wd_status: WifiDirectStatus,
    pub(super) wa_status: WifiAwareStatus,
    pub(super) handle: PrnsNodeHandle,
    pub(super) destination: DestinationHash,
    pub(super) node_page_destination: DestinationHash,
    pub(super) rpc_key: RpcAuthenticationKey,
    pub(super) ports: EnginePorts,
    pub(super) persistence: PersistenceHealth,
    pub(super) runtime: tokio::runtime::Handle,
    pub(super) configured: Arc<ConfiguredInterfaces>,
    pub(super) plan_context: PlanRuntimeContext,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EngineIdentitySnapshot {
    pub(crate) node_identity_hash: IdentityHash,
    pub(crate) ble_identity: BleIdentity,
    pub(crate) delivery_destination: DestinationHash,
    pub(crate) node_page_destination: DestinationHash,
}

#[derive(Clone)]
pub(super) struct PlatformLinks {
    pub(super) usb: AndroidUsbBridge,
    pub(super) ble: AndroidBleBridge,
    pub(super) wifi_direct: AndroidWifiDirectBridge,
    pub(super) wifi_aware: AndroidWifiAwareBridge,
    pub(super) service_discovery: AndroidServiceDiscoveryBridge,
}

impl PlatformLinks {
    fn new() -> Self {
        Self {
            usb: AndroidUsbBridge::new(),
            ble: AndroidBleBridge::new(),
            wifi_direct: AndroidWifiDirectBridge::new(),
            wifi_aware: AndroidWifiAwareBridge::new(),
            service_discovery: AndroidServiceDiscoveryBridge::new(),
        }
    }
}

struct EngineProcess {
    generation: u64,
    resources: Option<EngineResources>,
    platform: PlatformLinks,
    shutdown_tx: Option<oneshot::Sender<()>>,
    stopped_rx: Receiver<WorkerExit>,
    worker: Option<thread::JoinHandle<()>>,
}

struct EngineManager {
    state: MobileEngineState,
    last_failure: MobileEngineFailure,
    storage_dir: Option<PathBuf>,
    generation: u64,
    process: Option<EngineProcess>,
}

impl EngineManager {
    fn new() -> Self {
        Self {
            state: MobileEngineState::Stopped,
            last_failure: MobileEngineFailure::None,
            storage_dir: None,
            generation: 0,
            process: None,
        }
    }

    fn reap_finished(&mut self) {
        let finished = match self.process.as_ref() {
            Some(process) => match process.stopped_rx.try_recv() {
                Ok(exit) => Some(exit),
                Err(TryRecvError::Disconnected) => {
                    Some(WorkerExit::Failed(MobileEngineFailure::WorkerStopped))
                }
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        let Some(mut exit) = finished else {
            return;
        };
        let Some(mut process) = self.process.take() else {
            return;
        };
        if process
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            exit = WorkerExit::Failed(MobileEngineFailure::WorkerStopped);
        }
        match exit {
            WorkerExit::Stopped => {
                if self.state != MobileEngineState::Stopped {
                    self.state = MobileEngineState::Failed;
                    self.last_failure = MobileEngineFailure::WorkerStopped;
                }
            }
            WorkerExit::Failed(failure) => {
                self.state = MobileEngineState::Failed;
                self.last_failure = failure;
            }
        }
    }
}

static ENGINE_MANAGER: OnceLock<Mutex<EngineManager>> = OnceLock::new();

fn lock_manager() -> MutexGuard<'static, EngineManager> {
    ENGINE_MANAGER
        .get_or_init(|| Mutex::new(EngineManager::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineStartError {
    StorageConfiguration,
    PersistenceWrite,
    WorkerSpawn,
    RuntimeBuild,
    LocalListenerBind,
    RpcListenerBind,
    StartupTimeout,
    WorkerStopped,
}

impl EngineStartError {
    #[must_use]
    pub(crate) const fn failure(self) -> MobileEngineFailure {
        match self {
            Self::StorageConfiguration => MobileEngineFailure::StorageConfiguration,
            Self::PersistenceWrite => MobileEngineFailure::PersistenceWrite,
            Self::WorkerSpawn => MobileEngineFailure::WorkerSpawn,
            Self::RuntimeBuild => MobileEngineFailure::RuntimeBuild,
            Self::LocalListenerBind => MobileEngineFailure::LocalListenerBind,
            Self::RpcListenerBind => MobileEngineFailure::RpcListenerBind,
            Self::StartupTimeout => MobileEngineFailure::StartupTimeout,
            Self::WorkerStopped => MobileEngineFailure::WorkerStopped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineStopError {
    PersistenceWrite,
    ShutdownTimeout,
    WorkerStopped,
}

impl EngineStopError {
    #[must_use]
    pub(crate) const fn failure(self) -> MobileEngineFailure {
        match self {
            Self::PersistenceWrite => MobileEngineFailure::PersistenceWrite,
            Self::ShutdownTimeout => MobileEngineFailure::ShutdownTimeout,
            Self::WorkerStopped => MobileEngineFailure::WorkerStopped,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum WorkerExit {
    Stopped,
    Failed(MobileEngineFailure),
}

pub(crate) fn start(storage_dir: PathBuf) -> Result<(), EngineStartError> {
    start_with_ports(storage_dir, EnginePorts::ANDROID)
}

fn start_with_ports(storage_dir: PathBuf, ports: EnginePorts) -> Result<(), EngineStartError> {
    let storage_dir = prepare_storage_dir(storage_dir)?;
    let platform = PlatformLinks::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (stopped_tx, stopped_rx) = mpsc::channel();

    let generation = {
        let mut manager = lock_manager();
        manager.reap_finished();
        if let Some(configured) = manager.storage_dir.as_ref() {
            if configured != &storage_dir {
                manager.last_failure = MobileEngineFailure::StorageConfiguration;
                return Err(EngineStartError::StorageConfiguration);
            }
        }
        if manager.state == MobileEngineState::Running {
            return Ok(());
        }
        if manager.process.is_some() {
            return Err(EngineStartError::WorkerStopped);
        }
        manager.storage_dir = Some(storage_dir.clone());
        manager.generation = manager.generation.wrapping_add(1);
        manager.state = MobileEngineState::Starting;
        manager.last_failure = MobileEngineFailure::None;
        let generation = manager.generation;
        let worker_platform = platform.clone();
        let worker = thread::Builder::new()
            .name("hopspot-engine".into())
            .spawn(move || {
                worker::run(worker::WorkerInput {
                    storage_dir,
                    platform: worker_platform,
                    ready_tx,
                    shutdown_rx,
                    stopped_tx,
                    ports,
                })
            })
            .map_err(|_| {
                manager.state = MobileEngineState::Failed;
                manager.last_failure = MobileEngineFailure::WorkerSpawn;
                EngineStartError::WorkerSpawn
            })?;
        manager.process = Some(EngineProcess {
            generation,
            resources: None,
            platform,
            shutdown_tx: Some(shutdown_tx),
            stopped_rx,
            worker: Some(worker),
        });
        generation
    };

    let ready = ready_rx.recv_timeout(STARTUP_TIMEOUT);
    let mut manager = lock_manager();
    if manager.generation != generation {
        return Err(EngineStartError::WorkerStopped);
    }
    let Some(process) = manager.process.as_mut() else {
        return Err(EngineStartError::WorkerStopped);
    };
    if process.generation != generation {
        return Err(EngineStartError::WorkerStopped);
    }
    match ready {
        Ok(Ok(resources)) => {
            process.resources = Some(resources);
            manager.state = MobileEngineState::Running;
            manager.last_failure = MobileEngineFailure::None;
            Ok(())
        }
        Ok(Err(error)) => {
            manager.state = MobileEngineState::Failed;
            manager.last_failure = error.failure();
            Err(error)
        }
        Err(RecvTimeoutError::Timeout) => {
            if let Some(shutdown_tx) = process.shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
            manager.state = MobileEngineState::Failed;
            manager.last_failure = MobileEngineFailure::StartupTimeout;
            Err(EngineStartError::StartupTimeout)
        }
        Err(RecvTimeoutError::Disconnected) => {
            manager.state = MobileEngineState::Failed;
            manager.last_failure = MobileEngineFailure::WorkerStopped;
            Err(EngineStartError::WorkerStopped)
        }
    }
}

fn prepare_storage_dir(storage_dir: PathBuf) -> Result<PathBuf, EngineStartError> {
    if !storage_dir.is_absolute() {
        record_configuration_failure();
        return Err(EngineStartError::StorageConfiguration);
    }
    if std::fs::create_dir_all(&storage_dir).is_err() {
        record_configuration_failure();
        return Err(EngineStartError::StorageConfiguration);
    }
    storage_dir.canonicalize().map_err(|_| {
        record_configuration_failure();
        EngineStartError::StorageConfiguration
    })
}

fn record_configuration_failure() {
    let mut manager = lock_manager();
    if manager.state != MobileEngineState::Running {
        manager.state = MobileEngineState::Failed;
    }
    manager.last_failure = MobileEngineFailure::StorageConfiguration;
}

pub(crate) fn stop() -> Result<(), EngineStopError> {
    let mut process = {
        let mut manager = lock_manager();
        manager.reap_finished();
        let Some(process) = manager.process.take() else {
            manager.state = MobileEngineState::Stopped;
            return Ok(());
        };
        manager.generation = manager.generation.wrapping_add(1);
        process
    };

    if let Some(shutdown_tx) = process.shutdown_tx.take() {
        let _ = shutdown_tx.send(());
    }
    let exit = process.stopped_rx.recv_timeout(SHUTDOWN_TIMEOUT);
    match exit {
        Ok(exit) => finish_stopped_process(process, exit),
        Err(RecvTimeoutError::Disconnected) => finish_stopped_process(
            process,
            WorkerExit::Failed(MobileEngineFailure::WorkerStopped),
        ),
        Err(RecvTimeoutError::Timeout) => {
            let mut manager = lock_manager();
            manager.state = MobileEngineState::Failed;
            manager.last_failure = MobileEngineFailure::ShutdownTimeout;
            manager.process = Some(process);
            Err(EngineStopError::ShutdownTimeout)
        }
    }
}

fn finish_stopped_process(
    mut process: EngineProcess,
    mut exit: WorkerExit,
) -> Result<(), EngineStopError> {
    if process
        .worker
        .take()
        .is_some_and(|worker| worker.join().is_err())
    {
        exit = WorkerExit::Failed(MobileEngineFailure::WorkerStopped);
    }
    let mut manager = lock_manager();
    match exit {
        WorkerExit::Stopped => {
            manager.state = MobileEngineState::Stopped;
            Ok(())
        }
        WorkerExit::Failed(failure) => {
            manager.state = MobileEngineState::Failed;
            manager.last_failure = failure;
            if failure == MobileEngineFailure::PersistenceWrite {
                Err(EngineStopError::PersistenceWrite)
            } else {
                Err(EngineStopError::WorkerStopped)
            }
        }
    }
}

pub(crate) fn engine_state() -> MobileEngineState {
    let mut manager = lock_manager();
    manager.reap_finished();
    manager.state
}

pub(crate) fn last_failure() -> MobileEngineFailure {
    let mut manager = lock_manager();
    manager.reap_finished();
    manager.last_failure
}

pub(crate) fn interface_snapshots() -> Vec<InterfaceSnapshot> {
    let mut manager = lock_manager();
    manager.reap_finished();
    manager
        .process
        .as_ref()
        .and_then(|process| process.resources.as_ref())
        .map(|resources| resources.handle.interfaces())
        .unwrap_or_default()
}

pub(crate) fn runtime_health() -> Option<RuntimeHealth> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    Some(RuntimeHealth::from_snapshots(
        resources.started_at.elapsed(),
        &resources.handle.interfaces(),
    ))
}

pub(crate) fn persistence_snapshot() -> Option<PersistenceSnapshot> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    Some(resources.persistence.snapshot())
}

pub(crate) fn rpc_key_hex() -> Option<String> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    Some(hex(resources.rpc_key.as_bytes()))
}

pub(crate) fn sideband_join_config() -> Option<String> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    Some(sideband_join::render(&resources.rpc_key, resources.ports))
}

pub(crate) fn identity_snapshot() -> Option<EngineIdentitySnapshot> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    Some(EngineIdentitySnapshot {
        node_identity_hash: resources.node_identity_hash,
        ble_identity: resources.ble_identity,
        delivery_destination: resources.destination,
        node_page_destination: resources.node_page_destination,
    })
}

pub(crate) fn node_identity_hash_hex() -> Option<String> {
    identity_snapshot().map(|identity| hex(identity.node_identity_hash.as_bytes()))
}

pub(crate) fn ble_identity_hex() -> Option<String> {
    identity_snapshot().map(|identity| hex(identity.ble_identity.as_bytes()))
}

pub(crate) fn delivery_destination_hex() -> Option<String> {
    identity_snapshot().map(|identity| hex(identity.delivery_destination.as_bytes()))
}

pub(crate) fn node_page_destination_hex() -> Option<String> {
    identity_snapshot().map(|identity| hex(identity.node_page_destination.as_bytes()))
}

pub(crate) fn wifi_aware_failure_reason() -> Option<&'static str> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    resources.wa_status.failure_reason()
}

pub(crate) fn wifi_direct_failure_reason() -> Option<&'static str> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager.process.as_ref()?.resources.as_ref()?;
    resources.wd_status.failure_reason()
}

pub(crate) fn discovery_groups(
    interface: MobileDiscoveryInterface,
) -> Result<DiscoveryGroupSet, MobileDiscoveryGroupOutcome> {
    let mut manager = lock_manager();
    manager.reap_finished();
    let resources = manager
        .process
        .as_ref()
        .and_then(|process| process.resources.as_ref())
        .ok_or(MobileDiscoveryGroupOutcome::EngineUnavailable)?;
    Ok(match interface {
        MobileDiscoveryInterface::BluetoothAuto => resources.ble_status.discovery_groups(),
        MobileDiscoveryInterface::AutoWifi => resources.wifi_status.discovery_groups(),
    })
}

pub(crate) fn replace_discovery_groups(
    interface: MobileDiscoveryInterface,
    groups: DiscoveryGroupSet,
) -> MobileDiscoveryGroupOutcome {
    enum Status {
        Bluetooth(BluetoothAutoStatus),
        Wifi(AutoWifiStatus),
    }

    let (runtime, status) = {
        let mut manager = lock_manager();
        manager.reap_finished();
        let Some(resources) = manager
            .process
            .as_ref()
            .and_then(|process| process.resources.as_ref())
        else {
            return MobileDiscoveryGroupOutcome::EngineUnavailable;
        };
        let status = match interface {
            MobileDiscoveryInterface::BluetoothAuto => {
                Status::Bluetooth(resources.ble_status.clone())
            }
            MobileDiscoveryInterface::AutoWifi => Status::Wifi(resources.wifi_status.clone()),
        };
        (resources.runtime.clone(), status)
    };

    let unchanged = match &status {
        Status::Bluetooth(status) => status.discovery_groups() == groups,
        Status::Wifi(status) => status.discovery_groups() == groups,
    };
    if unchanged {
        return MobileDiscoveryGroupOutcome::Unchanged;
    }

    let (settled_tx, settled_rx) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        let outcome = match status {
            Status::Bluetooth(status) => status.replace_discovery_groups(groups).await,
            Status::Wifi(status) => status.replace_discovery_groups_and_wait(groups).await,
        };
        let _ = settled_tx.send(outcome);
    });
    match settled_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(DiscoveryGroupApplyOutcome::Applied) => MobileDiscoveryGroupOutcome::Applied,
        Ok(DiscoveryGroupApplyOutcome::Unchanged) => MobileDiscoveryGroupOutcome::Unchanged,
        Ok(DiscoveryGroupApplyOutcome::Failed) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            MobileDiscoveryGroupOutcome::ApplyFailed
        }
        Err(mpsc::RecvTimeoutError::Timeout) => MobileDiscoveryGroupOutcome::Busy,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

pub(crate) fn toggle_interface(id: InterfaceId) {
    let mut manager = lock_manager();
    manager.reap_finished();
    let Some(resources) = manager
        .process
        .as_ref()
        .and_then(|process| process.resources.as_ref())
    else {
        return;
    };
    if id == USB_INTERFACE_ID {
        resources.usb_status.toggle_enabled();
    } else if id == resources.wifi_status.id() {
        resources.wifi_status.toggle_enabled();
    } else if id.kind() == Some(InterfaceKind::BluetoothAuto) {
        resources.ble_status.toggle_enabled();
    } else if id.kind() == Some(InterfaceKind::WifiDirect) {
        resources.wd_status.toggle_enabled();
    } else if id.kind() == Some(InterfaceKind::WifiAware) {
        resources.wa_status.toggle_enabled();
    }
}

pub(crate) fn sleep_interfaces() {
    set_interfaces_enabled(false);
}

pub(crate) fn wake_interfaces() {
    set_interfaces_enabled(true);
}

fn set_interfaces_enabled(enabled: bool) {
    let mut manager = lock_manager();
    manager.reap_finished();
    let Some(resources) = manager
        .process
        .as_ref()
        .and_then(|process| process.resources.as_ref())
    else {
        return;
    };
    if enabled {
        resources.usb_status.enable();
        resources.wifi_status.enable();
        resources.ble_status.enable();
        resources.wd_status.enable();
        resources.wa_status.enable();
    } else {
        resources.usb_status.disable();
        resources.wifi_status.disable();
        resources.ble_status.disable();
        resources.wd_status.disable();
        resources.wa_status.disable();
    }
}

pub(crate) fn announce() {
    let mut manager = lock_manager();
    manager.reap_finished();
    let Some(resources) = manager
        .process
        .as_ref()
        .and_then(|process| process.resources.as_ref())
    else {
        return;
    };
    log::info!("hopspot: manual announce -> nomadnetwork.node on every interface");
    let _ = resources
        .handle
        .issue(PrnsCommand::AnnounceNow(AnnounceNow {
            destination: resources.node_page_destination,
            target: AnnounceTarget::AllInterfaces,
            app_data: AnnounceAppData::Registered,
        }));
}

const CONFIG_RELOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// The dashboard's view of the user-configured interfaces, as JSON (see `configured::describe`).
/// Readable while the engine is stopped, as long as it has been started once in this process.
pub(crate) fn configured_interfaces_json() -> Result<String, String> {
    let (storage_dir, running) = {
        let mut manager = lock_manager();
        manager.reap_finished();
        let storage_dir = manager
            .storage_dir
            .clone()
            .ok_or("the node has not been started")?;
        let running = manager
            .process
            .as_ref()
            .and_then(|process| process.resources.as_ref())
            .map(|resources| (resources.configured.clone(), resources.handle.interfaces()));
        (storage_dir, running)
    };
    configured::describe(
        &storage_dir,
        running
            .as_ref()
            .map(|(configured, snapshots)| (configured.as_ref(), snapshots.clone())),
    )
}

/// The interfaces the app runs itself (built-in radios, the shared-instance listener, and the
/// apps attached to it), as JSON for the dashboard. Fleet members of configured interfaces are
/// left to `configured_interfaces_json`.
pub(crate) fn built_in_interfaces_json() -> String {
    let (snapshots, configured_ids) = {
        let mut manager = lock_manager();
        manager.reap_finished();
        manager
            .process
            .as_ref()
            .and_then(|process| process.resources.as_ref())
            .map(|resources| {
                (
                    resources.handle.interface_inventory(),
                    resources.configured.attached_ids(),
                )
            })
            .unwrap_or_default()
    };
    let interfaces = snapshots
        .into_iter()
        .filter(|entry| !configured_ids.contains(&entry.snapshot.id))
        .filter(|entry| matches!(entry.snapshot.membership, Membership::Independent))
        .map(|entry| {
            let snapshot = entry.snapshot;
            let label = classify(snapshot.id)
                .map(|(_, label)| label.as_str().to_string())
                .unwrap_or_else(|| "Interface".to_string());
            serde_json::json!({
                "id": hex(snapshot.id.as_bytes()),
                "label": label,
                "name": entry.name,
                "kind": snapshot.id.kind().map(|kind| format!("{kind:?}")),
                "online": snapshot.connection.is_online(),
                "enabled": snapshot.connection != ConnectionState::Disabled,
                "connection": format!("{:?}", snapshot.connection).to_ascii_lowercase(),
                "toggleable": is_toggleable(snapshot.id),
                "rx_bytes": snapshot.rx_bytes,
                "tx_bytes": snapshot.tx_bytes,
            })
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(interfaces).to_string()
}

fn is_toggleable(id: InterfaceId) -> bool {
    id == USB_INTERFACE_ID
        || matches!(
            id.kind(),
            Some(
                InterfaceKind::AutoWifi
                    | InterfaceKind::BluetoothAuto
                    | InterfaceKind::WifiDirect
                    | InterfaceKind::WifiAware
            )
        )
}

/// Toggle a built-in radio by the hex id `built_in_interfaces_json` reported.
pub(crate) fn toggle_interface_hex(id_hex: &str) -> bool {
    let Some(id) = parse_interface_id(id_hex) else {
        return false;
    };
    if !is_toggleable(id) {
        return false;
    }
    toggle_interface(id);
    true
}

fn parse_interface_id(id_hex: &str) -> Option<InterfaceId> {
    let mut bytes = [0u8; 8];
    if id_hex.len() != bytes.len() * 2 {
        return None;
    }
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(id_hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(InterfaceId::new(bytes))
}

/// Apply one dashboard edit to the config file and, when the node is running, reload the
/// configured interfaces so the change takes effect without restarting the app.
pub(crate) fn apply_interface_edit(request: &str) -> Result<(), String> {
    let (storage_dir, running) = {
        let mut manager = lock_manager();
        manager.reap_finished();
        let storage_dir = manager
            .storage_dir
            .clone()
            .ok_or("the node has not been started")?;
        let running = manager
            .process
            .as_ref()
            .and_then(|process| process.resources.as_ref())
            .map(|resources| {
                (
                    resources.runtime.clone(),
                    resources.handle.clone(),
                    resources.configured.clone(),
                    resources.plan_context.clone(),
                )
            });
        (storage_dir, running)
    };
    configured::apply(&storage_dir, request)?;
    let Some((runtime, handle, configured, context)) = running else {
        return Ok(());
    };
    let (settled_tx, settled_rx) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        let result = configured.reload(&handle, &storage_dir, &context).await;
        let _ = settled_tx.send(result);
    });
    match settled_rx.recv_timeout(CONFIG_RELOAD_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err("saved; the node is still applying the change".to_string())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("saved, but the node stopped before applying it".to_string())
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalClientAdoptError {
    EngineUnavailable,
    Socket,
}

#[cfg(unix)]
impl LocalClientAdoptError {
    #[must_use]
    pub(crate) const fn code(self) -> i32 {
        match self {
            Self::EngineUnavailable => 1,
            Self::Socket => 2,
        }
    }
}

#[cfg(unix)]
static BINDER_CLIENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Attach one end of a socketpair handed over Binder as a shared-instance local client, the same
/// interface a loopback TCP client of the local listener gets. `uid` is the calling app's Linux
/// uid and only tags the channel, so each app's connection is distinguishable in snapshots.
#[cfg(unix)]
pub(crate) fn adopt_local_client(
    stream: std::os::unix::net::UnixStream,
    uid: u32,
) -> Result<(), LocalClientAdoptError> {
    let (handle, runtime) = {
        let mut manager = lock_manager();
        manager.reap_finished();
        let resources = manager
            .process
            .as_ref()
            .and_then(|process| process.resources.as_ref())
            .ok_or(LocalClientAdoptError::EngineUnavailable)?;
        (resources.handle.clone(), resources.runtime.clone())
    };
    stream
        .set_nonblocking(true)
        .map_err(|_| LocalClientAdoptError::Socket)?;
    let stream = {
        let _entered = runtime.enter();
        tokio::net::UnixStream::from_std(stream).map_err(|_| LocalClientAdoptError::Socket)?
    };
    let nth = BINDER_CLIENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let channel_tag = format!("binder:{uid}#{nth}").into_bytes();
    log::info!("hopspot: binder local client attached for uid {uid}");
    let _ = handle.add_interface(SharedInstanceClient::new(channel_tag, stream));
    Ok(())
}

pub(crate) fn usb_bridge() -> AndroidUsbBridge {
    lock_manager()
        .process
        .as_ref()
        .map(|process| process.platform.usb.clone())
        .unwrap_or_default()
}

pub(crate) fn ble_bridge() -> AndroidBleBridge {
    lock_manager()
        .process
        .as_ref()
        .map(|process| process.platform.ble.clone())
        .unwrap_or_default()
}

pub(crate) fn wd_bridge() -> AndroidWifiDirectBridge {
    lock_manager()
        .process
        .as_ref()
        .map(|process| process.platform.wifi_direct.clone())
        .unwrap_or_default()
}

pub(crate) fn wa_bridge() -> AndroidWifiAwareBridge {
    lock_manager()
        .process
        .as_ref()
        .map(|process| process.platform.wifi_aware.clone())
        .unwrap_or_default()
}

pub(crate) fn service_discovery_bridge() -> AndroidServiceDiscoveryBridge {
    lock_manager()
        .process
        .as_ref()
        .map(|process| process.platform.service_discovery.clone())
        .unwrap_or_default()
}

pub(crate) fn classify(id: InterfaceId) -> Option<(CardKind, CardLabel)> {
    if id == USB_INTERFACE_ID {
        return Some((CardKind::Usb, card_label("USB")));
    }
    match id.kind() {
        Some(InterfaceKind::AutoWifi) => Some((CardKind::Wifi, card_label("LAN"))),
        Some(InterfaceKind::LocalServer) => Some((CardKind::SharedInstance, card_label("Local"))),
        Some(InterfaceKind::LocalClient) => Some((CardKind::Peer, card_label("App"))),
        Some(InterfaceKind::BluetoothAuto) => Some((CardKind::Ble, card_label("BLE"))),
        Some(InterfaceKind::WifiDirect) => Some((CardKind::Wifi, card_label("Direct"))),
        Some(InterfaceKind::WifiAware) => Some((CardKind::Wifi, card_label("Aware"))),
        kind => {
            let bytes = id.as_bytes();
            let (card_kind, tag) = match kind {
                Some(InterfaceKind::BluetoothPeer) => (CardKind::Ble, "BLE"),
                Some(InterfaceKind::WifiDirectPeer) => (CardKind::Wifi, "WD"),
                Some(InterfaceKind::WifiAwarePeer) => (CardKind::Wifi, "P2P"),
                _ => (CardKind::Peer, "Peer"),
            };
            let mut label = CardLabel::new();
            let _ = write!(label, "{tag} {:02x}{:02x}", bytes[1], bytes[2]);
            Some((card_kind, label))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use personal_hopspot_core::card_label_max_chars;

    #[test]
    fn wifi_direct_is_presented_as_a_lab_transport() {
        let id = InterfaceId::from_channel_tag(InterfaceKind::WifiDirect, b"wifi-direct");
        let (_, label) = classify(id).expect("Wi-Fi Direct has a card");
        assert_eq!(label.as_str(), "Direct");
    }

    #[test]
    fn every_android_interface_label_fits_its_card() {
        let kinds = [
            InterfaceKind::AutoWifi,
            InterfaceKind::LocalServer,
            InterfaceKind::LocalClient,
            InterfaceKind::BluetoothAuto,
            InterfaceKind::WifiDirect,
            InterfaceKind::WifiAware,
            InterfaceKind::BluetoothPeer,
            InterfaceKind::WifiDirectPeer,
            InterfaceKind::WifiAwarePeer,
        ];
        for kind in kinds {
            let id = InterfaceId::from_channel_tag(kind, b"label-fit");
            let (card_kind, label) = classify(id).expect("known interface has a card");
            assert!(
                label.chars().count() <= card_label_max_chars(card_kind),
                "{kind:?} label {label:?} exceeds its card"
            );
        }
    }

    #[cfg(unix)]
    fn local_client_count() -> u32 {
        runtime_health().map_or(0, |health| health.local_client_count)
    }

    #[cfg(unix)]
    fn wait_for_local_client_count(expected: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while local_client_count() != expected {
            assert!(
                Instant::now() < deadline,
                "local client count stayed at {} instead of {expected}",
                local_client_count()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    fn assert_binder_local_client_attaches_and_detaches() {
        let before = local_client_count();
        let nth = BINDER_CLIENT_SEQUENCE.load(Ordering::Relaxed);
        let expected_id = InterfaceId::from_channel_tag(
            InterfaceKind::LocalClient,
            format!("binder:10123#{nth}").as_bytes(),
        );
        let (peer, hub_end) = std::os::unix::net::UnixStream::pair().unwrap();
        adopt_local_client(hub_end, 10_123).unwrap();
        wait_for_local_client_count(before + 1);
        assert!(
            interface_snapshots()
                .iter()
                .any(|snapshot| snapshot.id == expected_id),
            "the adopted socket is a local-client interface"
        );
        drop(peer);
        wait_for_local_client_count(before);
    }

    #[test]
    fn engine_stops_and_restarts_with_durable_identity_and_runtime_state() {
        let storage_dir =
            std::env::temp_dir().join(format!("personal-hopspot-android-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&storage_dir);

        start_with_ports(storage_dir.clone(), EnginePorts::EPHEMERAL).unwrap();
        assert_eq!(engine_state(), MobileEngineState::Running);
        let first_key = rpc_key_hex().unwrap();
        assert_eq!(first_key.len(), 64);
        let first_sideband_config = sideband_join_config().unwrap();
        assert!(first_sideband_config.contains(&format!("rpc_key = {first_key}")));
        let first_identity = identity_snapshot().unwrap();
        assert_eq!(persistence_snapshot().unwrap().successful_flushes, 1);
        #[cfg(unix)]
        assert_binder_local_client_attaches_and_detaches();
        stop().unwrap();
        #[cfg(unix)]
        {
            let (_peer, hub_end) = std::os::unix::net::UnixStream::pair().unwrap();
            assert_eq!(
                adopt_local_client(hub_end, 10_123),
                Err(LocalClientAdoptError::EngineUnavailable)
            );
        }
        assert_eq!(engine_state(), MobileEngineState::Stopped);
        for region in ["timebase", "routing_table", "tunnels", "known_destinations"] {
            assert!(storage_dir.join("prns").join(region).is_file());
        }

        start_with_ports(storage_dir.clone(), EnginePorts::EPHEMERAL).unwrap();
        assert_eq!(engine_state(), MobileEngineState::Running);
        assert_eq!(rpc_key_hex().unwrap(), first_key);
        assert_eq!(sideband_join_config().unwrap(), first_sideband_config);
        assert_eq!(identity_snapshot().unwrap(), first_identity);
        let persistence = persistence_snapshot().unwrap();
        assert_eq!(persistence.restore.refused, 0);
        assert_eq!(persistence.restore.dropped, 0);
        assert_eq!(persistence.restore.ratchets, 1);
        assert_eq!(persistence.successful_flushes, 1);
        stop().unwrap();
        assert_eq!(engine_state(), MobileEngineState::Stopped);

        std::fs::write(storage_dir.join("prns").join("routing_table"), b"damaged").unwrap();
        start_with_ports(storage_dir.clone(), EnginePorts::EPHEMERAL).unwrap();
        let persistence = persistence_snapshot().unwrap();
        assert_eq!(persistence.restore.refused, 1);
        assert_eq!(persistence.successful_flushes, 1);
        stop().unwrap();
        assert_eq!(engine_state(), MobileEngineState::Stopped);

        std::fs::remove_dir_all(storage_dir.join("prns")).unwrap();
        std::fs::write(storage_dir.join("prns"), b"not a directory").unwrap();
        assert_eq!(
            start_with_ports(storage_dir.clone(), EnginePorts::EPHEMERAL),
            Err(EngineStartError::StorageConfiguration)
        );
        assert_eq!(last_failure(), MobileEngineFailure::StorageConfiguration);
        assert_eq!(stop(), Err(EngineStopError::WorkerStopped));
        std::fs::remove_dir_all(storage_dir).unwrap();
    }
}
