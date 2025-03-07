use crate::account::AccountToken;
#[cfg(target_os = "android")]
use jnix::IntoJava;
use serde::{Deserialize, Serialize};
use std::fmt;
use talpid_types::net::wireguard::PublicKey;

/// UUID for a device.
pub type DeviceId = String;

/// Human-readable device identifier.
pub type DeviceName = String;

/// Contains data for a device returned by the API.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[cfg_attr(target_os = "android", derive(IntoJava))]
#[cfg_attr(target_os = "android", jnix(package = "net.mullvad.mullvadvpn.model"))]
pub struct Device {
    pub id: DeviceId,
    pub name: DeviceName,
    #[cfg_attr(target_os = "android", jnix(map = "|key| *key.as_bytes()"))]
    pub pubkey: PublicKey,
    pub ports: Vec<DevicePort>,
}

impl Eq for Device {}

impl Device {
    /// Return name with each word capitalized: "Happy Seagull" instead of "happy seagull"
    pub fn pretty_name(&self) -> String {
        self.name
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<String>>()
            .join(" ")
    }

    pub fn eq_id(&self, other: &Device) -> bool {
        self.id == other.id
    }
}

/// Ports associated with a device.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[cfg_attr(target_os = "android", derive(IntoJava))]
#[cfg_attr(target_os = "android", jnix(package = "net.mullvad.mullvadvpn.model"))]
pub struct DevicePort {
    /// Port identifier.
    pub id: String,
}

impl fmt::Display for DevicePort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

/// A [Device] and its associated account token.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[cfg_attr(target_os = "android", derive(IntoJava))]
#[cfg_attr(target_os = "android", jnix(package = "net.mullvad.mullvadvpn.model"))]
pub struct AccountAndDevice {
    pub account_token: AccountToken,
    pub device: Device,
}

impl AccountAndDevice {
    pub fn new(account_token: AccountToken, device: Device) -> Self {
        Self {
            account_token,
            device,
        }
    }
}

/// Emitted when logging in or out of an account, or when the device changes.
#[derive(Clone, Debug)]
#[cfg_attr(target_os = "android", derive(IntoJava))]
#[cfg_attr(target_os = "android", jnix(package = "net.mullvad.mullvadvpn.model"))]
pub struct DeviceEvent {
    /// Device that was affected.
    pub device: Option<AccountAndDevice>,
    /// Indicates whether the change was initiated remotely or by the daemon.
    pub remote: bool,
}

impl DeviceEvent {
    pub fn new(device: Option<AccountAndDevice>, remote: bool) -> DeviceEvent {
        DeviceEvent { device, remote }
    }

    pub fn from_device(device: AccountAndDevice, remote: bool) -> DeviceEvent {
        DeviceEvent {
            device: Some(device),
            remote,
        }
    }

    pub fn revoke(remote: bool) -> Self {
        Self {
            device: None,
            remote,
        }
    }
}

/// Emitted when a device is removed using the `RemoveDevice` RPC.
/// This is not sent by a normal logout or when it is revoked remotely.
#[derive(Clone, Debug)]
#[cfg_attr(target_os = "android", derive(IntoJava))]
#[cfg_attr(target_os = "android", jnix(package = "net.mullvad.mullvadvpn.model"))]
pub struct RemoveDeviceEvent {
    pub account_token: AccountToken,
    pub removed_device: Device,
    pub new_devices: Vec<Device>,
}
