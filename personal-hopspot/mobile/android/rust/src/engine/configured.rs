//! User-configured interfaces: a Reticulum-style `config` file in the engine's storage directory,
//! edited from the dashboard and hot-reloaded into the running node. The radios Android drives
//! itself (Wi-Fi Auto, Bluetooth LE Auto, USB Auto, Wi-Fi Direct/Aware) stay built in; this file
//! carries everything else a desktop `prnsd` config would, such as TCP, UDP, Backbone, I2P, and
//! WebSocket interfaces.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use personal_rns::config::editing::{
    ConfigEdit, ConfigFile, InterfaceDefinition, InterfaceName, InterfaceSetting,
    InterfaceSettingCategory, InterfaceSettingKey, InterfaceSettingTier, InterfaceSettingValue,
};
use personal_rns::config::{parse_and_plan_named, InterfaceKind as ConfiguredKind, PlannedMedium};
use personal_rns::interfaces::{InterfaceId, InterfaceSnapshot, Membership};
use personal_rns::runtime::PrnsNodeHandle;
use personal_rns::{attach_plan_with_context, PlanAttachments, PlanOutcome, PlanRuntimeContext};
use serde_json::{json, Map, Value};

pub(crate) const CONFIG_FILE_NAME: &str = "config";
const EMPTY_CONFIG: &str = "[interfaces]\n";

/// Interface types the dashboard offers. The platform-driven auto interfaces are excluded because
/// the app already runs them; a second copy from the config file would fight over the same radio.
pub(crate) const DASHBOARD_TYPES: &[ConfiguredKind] = &[
    ConfiguredKind::TcpClient,
    ConfiguredKind::TcpServer,
    ConfiguredKind::Udp,
    ConfiguredKind::BackboneClient,
    ConfiguredKind::Backbone,
    ConfiguredKind::I2p,
    ConfiguredKind::PrnsWebSocketClient,
    ConfiguredKind::PrnsWebSocketServer,
    ConfiguredKind::Rnode,
];

#[derive(Debug, Clone)]
enum Outcome {
    Up(InterfaceId),
    Failed(String),
}

#[derive(Default)]
pub(crate) struct ConfiguredInterfaces {
    attachments: tokio::sync::Mutex<PlanAttachments>,
    outcomes: Mutex<BTreeMap<String, Outcome>>,
}

impl ConfiguredInterfaces {
    /// Detach whatever the previous config stood up and attach the file's current interfaces.
    /// A config that does not parse leaves nothing attached and reports why.
    pub(super) async fn reload(
        &self,
        handle: &PrnsNodeHandle,
        storage_dir: &Path,
        context: &PlanRuntimeContext,
    ) -> Result<(), String> {
        let mut attachments = self.attachments.lock().await;
        core::mem::take(&mut *attachments).detach(handle).await;
        lock(&self.outcomes).clear();

        let source = read_source(storage_dir)?;
        let mut plan = parse_and_plan_named(CONFIG_FILE_NAME, &source)
            .map_err(|errors| errors.to_string())?
            .value;
        let mut outcomes = BTreeMap::new();
        plan.interfaces
            .retain(|interface| match built_in_reason(&interface.medium) {
                Some(reason) => {
                    outcomes.insert(interface.name.clone(), Outcome::Failed(reason.to_string()));
                    false
                }
                None => true,
            });
        *attachments = attach_plan_with_context(handle, &plan, context, &mut |outcome| {
            match outcome {
                PlanOutcome::Up { interface, id } => {
                    log::info!("hopspot: configured interface up: {}", interface.name);
                    outcomes.insert(interface.name.clone(), Outcome::Up(id));
                }
                PlanOutcome::Failed { interface, error } => {
                    log::warn!(
                        "hopspot: configured interface {} failed: {error}",
                        interface.name
                    );
                    outcomes.insert(interface.name.clone(), Outcome::Failed(error.to_string()));
                }
            }
        })
        .await;
        *lock(&self.outcomes) = outcomes;
        Ok(())
    }

    fn outcomes(&self) -> BTreeMap<String, Outcome> {
        lock(&self.outcomes).clone()
    }

    /// Ids of the interfaces the config file currently has attached.
    pub(super) fn attached_ids(&self) -> Vec<InterfaceId> {
        lock(&self.outcomes)
            .values()
            .filter_map(|outcome| match outcome {
                Outcome::Up(id) => Some(*id),
                Outcome::Failed(_) => None,
            })
            .collect()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn built_in_reason(medium: &PlannedMedium) -> Option<&'static str> {
    match medium {
        PlannedMedium::AutoWifi(_) => Some("Wi-Fi Auto is built into the app; toggle it there"),
        PlannedMedium::PrnsUsbAuto => Some("USB Auto is built into the app; toggle it there"),
        PlannedMedium::PrnsBluetoothAuto(_) => {
            Some("Bluetooth LE Auto is built into the app; toggle it there")
        }
        _ => None,
    }
}

fn config_path(storage_dir: &Path) -> std::path::PathBuf {
    storage_dir.join(CONFIG_FILE_NAME)
}

fn read_source(storage_dir: &Path) -> Result<String, String> {
    ConfigFile::load(config_path(storage_dir), EMPTY_CONFIG)
        .map(|file| file.document().source().to_string())
        .map_err(|error| error.to_string())
}

/// The configured interfaces as JSON for the dashboard, each joined with what the running node
/// made of it: `up` with live vitals, `failed` with the reason, or `disabled`/`stopped`.
pub(super) fn describe(
    storage_dir: &Path,
    state: Option<(&ConfiguredInterfaces, Vec<InterfaceSnapshot>)>,
) -> Result<String, String> {
    let file = ConfigFile::load(config_path(storage_dir), EMPTY_CONFIG)
        .map_err(|error| error.to_string())?;
    let (outcomes, snapshots) = match state {
        Some((configured, snapshots)) => (Some(configured.outcomes()), snapshots),
        None => (None, Vec::new()),
    };
    let interfaces = file
        .document()
        .interfaces()
        .into_iter()
        .map(|interface| {
            let name = interface.name().as_str().to_string();
            let enabled = interface.enabled().unwrap_or(true);
            let mut settings = Map::new();
            for setting in interface.settings() {
                let key = setting.spec().key().canonical().as_str();
                if key == "interface_enabled" || key == "type" {
                    continue;
                }
                settings.insert(key.to_string(), Value::String(setting.value().to_string()));
            }
            let mut entry = json!({
                "name": name,
                "type": interface.configured_type().unwrap_or_default(),
                "enabled": enabled,
                "settings": settings,
            });
            let status = match outcomes.as_ref().map(|outcomes| outcomes.get(&name)) {
                None => json!({ "state": "stopped" }),
                Some(_) if !enabled => json!({ "state": "disabled" }),
                Some(None) => json!({ "state": "pending" }),
                Some(Some(Outcome::Failed(reason))) => json!({
                    "state": "failed",
                    "error": reason,
                }),
                Some(Some(Outcome::Up(id))) => live_status(*id, &snapshots),
            };
            entry["status"] = status;
            entry
        })
        .collect::<Vec<_>>();
    let types = DASHBOARD_TYPES
        .iter()
        .map(|kind| type_form(*kind))
        .collect::<Vec<_>>();
    Ok(json!({
        "path": file.path().display().to_string(),
        "types": types,
        "interfaces": interfaces,
    })
    .to_string())
}

/// The form the dashboard shows for one interface type: its standard connectivity and access
/// settings, described by the same catalog `prnsd`'s config editor uses.
fn type_form(kind: ConfiguredKind) -> Value {
    let mut specs = kind
        .setting_specs()
        .into_iter()
        .filter(|spec| spec.is_supported(kind))
        .filter(|spec| spec.tier() == InterfaceSettingTier::Standard)
        .filter(|spec| {
            matches!(
                spec.category(),
                InterfaceSettingCategory::Connectivity
                    | InterfaceSettingCategory::Access
                    | InterfaceSettingCategory::Radio
            ) || spec.key().as_str() == "interface_mode"
        })
        .filter(|spec| spec.key().canonical() == spec.key())
        .collect::<Vec<_>>();
    specs.sort_by_key(|spec| (spec.required_hint(kind).is_none(), spec.category()));
    let settings = specs
        .into_iter()
        .map(|spec| {
            // Android cannot open serial devices, so an RNode is reached over Bluetooth LE
            // (through the app's GATT bridge) or its Wi-Fi TCP port.
            let hint = if kind == ConfiguredKind::Rnode && spec.key().as_str() == "port" {
                "ble://  ·  ble://RNode A1B2  ·  tcp://192.168.4.1"
            } else {
                spec.default_hint(kind).unwrap_or_else(|| spec.accepted(kind))
            };
            json!({
                "key": spec.key().as_str(),
                "label": spec.label(),
                "description": spec.description(),
                "required": spec.required_hint(kind).is_some(),
                "hint": hint,
                "input": format!("{:?}", spec.input_kind(kind)).to_ascii_lowercase(),
                "secret": spec.key().is_secret(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "name": kind.canonical_name(),
        "settings": settings,
    })
}

fn live_status(id: InterfaceId, snapshots: &[InterfaceSnapshot]) -> Value {
    let own = snapshots.iter().find(|snapshot| snapshot.id == id);
    let members = snapshots
        .iter()
        .filter(|snapshot| {
            matches!(snapshot.membership, Membership::FleetMember { supervisor_id } if supervisor_id == id)
        })
        .collect::<Vec<_>>();
    let online = own.is_some_and(|snapshot| snapshot.connection.is_online())
        || members
            .iter()
            .any(|snapshot| snapshot.connection.is_online());
    let mut rx_bytes = own.map_or(0, |snapshot| snapshot.rx_bytes);
    let mut tx_bytes = own.map_or(0, |snapshot| snapshot.tx_bytes);
    if own.is_none() {
        rx_bytes = members.iter().map(|snapshot| snapshot.rx_bytes).sum();
        tx_bytes = members.iter().map(|snapshot| snapshot.tx_bytes).sum();
    }
    json!({
        "state": "up",
        "online": online,
        "connection": own.map(|snapshot| format!("{:?}", snapshot.connection).to_ascii_lowercase()),
        "detail": own.and_then(|snapshot| snapshot.failure_reason),
        "peers": members.len(),
        "rx_bytes": rx_bytes,
        "tx_bytes": tx_bytes,
    })
}

/// Apply one dashboard edit to the config file. The edit is validated against the full config
/// before anything is written, so a rejected edit leaves the file untouched.
///
/// Accepted shapes:
/// - `{"op":"upsert","original":null|"<name>","name":"<name>","type":"<type>","enabled":true,"settings":{"key":"value"}}`
/// - `{"op":"remove","name":"<name>"}`
/// - `{"op":"set_enabled","name":"<name>","enabled":false}`
pub(super) fn apply(storage_dir: &Path, request: &str) -> Result<(), String> {
    let request: Value =
        serde_json::from_str(request).map_err(|error| format!("invalid request: {error}"))?;
    let op = request["op"].as_str().ok_or("missing op")?;
    let edit = match op {
        "upsert" => {
            let definition = definition(&request)?;
            match request["original"].as_str() {
                Some(original) => ConfigEdit::Replace {
                    current: interface_name(original)?,
                    replacement: definition,
                },
                None => ConfigEdit::Add(definition),
            }
        }
        "remove" => ConfigEdit::Remove(interface_name(
            request["name"].as_str().ok_or("missing name")?,
        )?),
        "set_enabled" => ConfigEdit::SetEnabled {
            name: interface_name(request["name"].as_str().ok_or("missing name")?)?,
            enabled: request["enabled"].as_bool().ok_or("missing enabled")?,
        },
        other => return Err(format!("unknown op {other}")),
    };
    let file = ConfigFile::load(config_path(storage_dir), EMPTY_CONFIG)
        .map_err(|error| error.to_string())?;
    let edited = file
        .document()
        .edit_named(CONFIG_FILE_NAME, &edit)
        .map_err(|error| error.to_string())?;
    file.write(&edited).map_err(|error| error.to_string())?;
    Ok(())
}

fn interface_name(value: &str) -> Result<InterfaceName, String> {
    InterfaceName::new(value.trim()).map_err(|error| error.to_string())
}

fn definition(request: &Value) -> Result<InterfaceDefinition, String> {
    let name = interface_name(request["name"].as_str().ok_or("missing name")?)?;
    let type_name = request["type"].as_str().ok_or("missing type")?;
    let kind = ConfiguredKind::parse(type_name)
        .filter(|kind| DASHBOARD_TYPES.contains(kind))
        .ok_or_else(|| format!("{type_name} cannot be added from the dashboard"))?;
    let enabled = request["enabled"].as_bool().unwrap_or(true);
    let mut settings = Vec::new();
    if let Some(values) = request["settings"].as_object() {
        for (key, value) in values {
            let Some(text) = value.as_str() else {
                return Err(format!("{key} must be text"));
            };
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let key = InterfaceSettingKey::parse(key)
                .ok_or_else(|| format!("{key} is not an interface setting"))?;
            let value = if text.contains(',') {
                InterfaceSettingValue::List(
                    text.split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            } else {
                InterfaceSettingValue::Text(text.to_string())
            };
            settings.push(InterfaceSetting::new(key, value));
        }
    }
    InterfaceDefinition::new_named_with_rnode_multi_radios(
        CONFIG_FILE_NAME,
        name,
        kind,
        enabled,
        settings,
        Vec::new(),
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hopspot-configured-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn dashboard_edits_round_trip_through_the_config_file() {
        let dir = temp_dir("round-trip");
        apply(
            &dir,
            r#"{"op":"upsert","original":null,"name":"Home server","type":"TCPClientInterface","enabled":true,"settings":{"target_host":"rns.example.org","target_port":"4242"}}"#,
        )
        .expect("add");
        let listed: Value = serde_json::from_str(&describe(&dir, None).expect("describe"))
            .expect("json");
        let interface = &listed["interfaces"][0];
        assert_eq!(interface["name"], "Home server");
        assert_eq!(interface["type"], "TCPClientInterface");
        assert_eq!(interface["settings"]["target_host"], "rns.example.org");
        assert_eq!(interface["settings"]["target_port"], "4242");
        assert_eq!(interface["status"]["state"], "stopped");

        apply(&dir, r#"{"op":"set_enabled","name":"Home server","enabled":false}"#)
            .expect("disable");
        let listed: Value = serde_json::from_str(&describe(&dir, None).expect("describe"))
            .expect("json");
        assert_eq!(listed["interfaces"][0]["enabled"], false);

        apply(&dir, r#"{"op":"remove","name":"Home server"}"#).expect("remove");
        let listed: Value = serde_json::from_str(&describe(&dir, None).expect("describe"))
            .expect("json");
        assert_eq!(listed["interfaces"].as_array().map(Vec::len), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_edits_leave_the_file_untouched() {
        let dir = temp_dir("invalid");
        let error = apply(
            &dir,
            r#"{"op":"upsert","original":null,"name":"Broken","type":"TCPClientInterface","settings":{"target_port":"not-a-port"}}"#,
        )
        .expect_err("rejected");
        assert!(!error.is_empty());
        assert!(!config_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn built_in_radios_cannot_be_added_from_the_dashboard() {
        let dir = temp_dir("built-in");
        let error = apply(
            &dir,
            r#"{"op":"upsert","original":null,"name":"LAN","type":"AutoInterface","settings":{}}"#,
        )
        .expect_err("rejected");
        assert!(error.contains("dashboard"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
