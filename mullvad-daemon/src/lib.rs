#![deny(rust_2018_idioms)]
#![recursion_limit = "512"]

#[macro_use]
extern crate serde;

pub mod account_history;
mod api;
pub mod device;
mod dns;
pub mod exception_logging;
#[cfg(target_os = "macos")]
pub mod exclusion_gid;
mod geoip;
pub mod logging;
#[cfg(not(target_os = "android"))]
pub mod management_interface;
mod migrations;
#[cfg(not(target_os = "android"))]
pub mod rpc_uniqueness_check;
pub mod runtime;
pub mod settings;
mod target_state;
pub mod version;
mod version_check;

use crate::target_state::PersistentTargetState;
use device::{PrivateAccountAndDevice, PrivateDeviceEvent};
use futures::{
    channel::{mpsc, oneshot},
    future::{abortable, AbortHandle, Future},
    StreamExt,
};
use mullvad_api::availability::ApiAvailabilityHandle;
use mullvad_relay_selector::{
    updater::{RelayListUpdater, RelayListUpdaterHandle},
    RelaySelector, SelectedBridge, SelectedObfuscator, SelectedRelay, SelectorConfig,
};
use mullvad_types::{
    account::{AccountData, AccountToken, VoucherSubmission},
    device::{AccountAndDevice, Device, DeviceEvent, DeviceId, RemoveDeviceEvent},
    endpoint::MullvadEndpoint,
    location::GeoIpLocation,
    relay_constraints::{BridgeSettings, BridgeState, ObfuscationSettings, RelaySettingsUpdate},
    relay_list::{Relay, RelayList},
    settings::{DnsOptions, Settings},
    states::{TargetState, TunnelState},
    version::{AppVersion, AppVersionInfo},
    wireguard::{PublicKey, RotationInterval},
};
use settings::SettingsPersister;
#[cfg(target_os = "android")]
use std::os::unix::io::RawFd;
#[cfg(not(target_os = "android"))]
use std::path::Path;
#[cfg(target_os = "windows")]
use std::{collections::HashSet, ffi::OsString};
use std::{
    marker::PhantomData,
    mem,
    path::PathBuf,
    pin::Pin,
    sync::{mpsc as sync_mpsc, Arc, Weak},
    time::Duration,
};
#[cfg(any(target_os = "linux", windows))]
use talpid_core::split_tunnel;
use talpid_core::{
    mpsc::Sender,
    tunnel_state_machine::{self, TunnelCommand, TunnelParametersGenerator},
};
#[cfg(target_os = "android")]
use talpid_types::android::AndroidContext;
#[cfg(not(target_os = "android"))]
use talpid_types::net::openvpn;
use talpid_types::{
    net::{wireguard, TunnelEndpoint, TunnelParameters, TunnelType},
    tunnel::{ErrorStateCause, ParameterGenerationError, TunnelStateTransition},
    ErrorExt,
};
#[cfg(not(target_os = "android"))]
use tokio::fs;
use tokio::io;

/// Delay between generating a new WireGuard key and reconnecting
const WG_RECONNECT_DELAY: Duration = Duration::from_secs(4 * 60);

pub type ResponseTx<T, E> = oneshot::Sender<Result<T, E>>;

#[derive(err_derive::Error, Debug)]
#[error(no_from)]
pub enum Error {
    #[error(display = "Failed to send command to daemon because it is not running")]
    DaemonUnavailable,

    #[error(display = "Unable to initialize network event loop")]
    InitIoEventLoop(#[error(source)] io::Error),

    #[error(display = "Unable to create RPC client")]
    InitRpcFactory(#[error(source)] mullvad_api::Error),

    #[error(display = "REST request failed")]
    RestError(#[error(source)] mullvad_api::rest::Error),

    #[error(display = "API availability check failed")]
    ApiCheckError(#[error(source)] mullvad_api::availability::Error),

    #[error(display = "Unable to load account history")]
    LoadAccountHistory(#[error(source)] account_history::Error),

    #[error(display = "Failed to start account manager")]
    LoadAccountManager(#[error(source)] device::Error),

    #[error(display = "Failed to log in to account")]
    LoginError(#[error(source)] device::Error),

    #[error(display = "Failed to log out of account")]
    LogoutError(#[error(source)] device::Error),

    #[error(display = "Failed to rotate WireGuard key")]
    KeyRotationError(#[error(source)] device::Error),

    #[error(display = "Failed to list devices")]
    ListDevicesError(#[error(source)] device::Error),

    #[error(display = "Failed to remove device")]
    RemoveDeviceError(#[error(source)] device::Error),

    #[error(display = "Failed to update device")]
    UpdateDeviceError(#[error(source)] device::Error),

    #[cfg(target_os = "linux")]
    #[error(display = "Unable to initialize split tunneling")]
    InitSplitTunneling(#[error(source)] split_tunnel::Error),

    #[cfg(windows)]
    #[error(display = "Split tunneling error")]
    SplitTunnelError(#[error(source)] split_tunnel::Error),

    #[error(display = "An account is already set")]
    AlreadyLoggedIn,

    #[error(display = "No wireguard private key available")]
    NoKeyAvailable,

    #[error(display = "No bridge available")]
    NoBridgeAvailable,

    #[error(display = "No matching entry relay was found")]
    NoEntryRelayAvailable,

    #[error(display = "No account token is set")]
    NoAccountToken,

    #[error(display = "No account history available for the token")]
    NoAccountTokenHistory,

    #[error(display = "Settings error")]
    SettingsError(#[error(source)] settings::Error),

    #[error(display = "Account history error")]
    AccountHistory(#[error(source)] account_history::Error),

    #[error(display = "Failed to clear cache directory")]
    ClearCacheError,

    #[error(display = "Failed to clear logs directory")]
    ClearLogsError,

    #[error(display = "Failed to clear account history")]
    ClearAccountHistoryError(#[error(source)] account_history::Error),

    #[error(display = "Failed to clear settings")]
    ClearSettingsError(#[error(source)] settings::Error),

    #[error(display = "Tunnel state machine error")]
    TunnelError(#[error(source)] tunnel_state_machine::Error),

    #[error(display = "Failed to remove directory {}", _0)]
    RemoveDirError(String, #[error(source)] io::Error),

    #[error(display = "Failed to create directory {}", _0)]
    CreateDirError(String, #[error(source)] io::Error),

    #[error(display = "Failed to get path")]
    PathError(#[error(source)] mullvad_paths::Error),

    #[cfg(target_os = "windows")]
    #[error(display = "Failed to get file type info")]
    FileTypeError(#[error(source)] io::Error),

    #[cfg(target_os = "windows")]
    #[error(display = "Failed to get dir entry")]
    FileEntryError(#[error(source)] io::Error),

    #[cfg(target_os = "windows")]
    #[error(display = "Failed to read dir entries")]
    ReadDirError(#[error(source)] io::Error),

    #[cfg(target_os = "macos")]
    #[error(display = "Failed to set exclusion group")]
    GroupIdError(#[error(source)] io::Error),
}

/// Enum representing commands that can be sent to the daemon.
pub enum DaemonCommand {
    /// Set target state. Does nothing if the daemon already has the state that is being set.
    SetTargetState(oneshot::Sender<bool>, TargetState),
    /// Reconnect the tunnel, if one is connecting/connected.
    Reconnect(oneshot::Sender<bool>),
    /// Request the current state.
    GetState(oneshot::Sender<TunnelState>),
    /// Get the current geographical location.
    GetCurrentLocation(oneshot::Sender<Option<GeoIpLocation>>),
    CreateNewAccount(ResponseTx<String, Error>),
    /// Request the metadata for an account.
    GetAccountData(
        ResponseTx<AccountData, mullvad_api::rest::Error>,
        AccountToken,
    ),
    /// Request www auth token for an account
    GetWwwAuthToken(ResponseTx<String, Error>),
    /// Submit voucher to add time to the current account. Returns time added in seconds
    SubmitVoucher(ResponseTx<VoucherSubmission, Error>, String),
    /// Request account history
    GetAccountHistory(oneshot::Sender<Option<AccountToken>>),
    /// Remove the last used account, if there is one
    ClearAccountHistory(ResponseTx<(), Error>),
    /// Get the list of countries and cities where there are relays.
    GetRelayLocations(oneshot::Sender<RelayList>),
    /// Trigger an asynchronous relay list update. This returns before the relay list is actually
    /// updated.
    UpdateRelayLocations,
    /// Log in with a given account and create a new device.
    LoginAccount(ResponseTx<(), Error>, AccountToken),
    /// Log out of the current account and remove the device, if they exist.
    LogoutAccount(ResponseTx<(), Error>),
    /// Return the current device configuration, if there is one.
    GetDevice(ResponseTx<Option<AccountAndDevice>, Error>),
    /// Update/check the current device, if there is one.
    UpdateDevice(ResponseTx<(), Error>),
    /// Return all the devices for a given account token.
    ListDevices(ResponseTx<Vec<Device>, Error>, AccountToken),
    /// Remove device from a given account.
    RemoveDevice(ResponseTx<(), Error>, AccountToken, DeviceId),
    /// Place constraints on the type of tunnel and relay
    UpdateRelaySettings(ResponseTx<(), settings::Error>, RelaySettingsUpdate),
    /// Set the allow LAN setting.
    SetAllowLan(ResponseTx<(), settings::Error>, bool),
    /// Set the beta program setting.
    SetShowBetaReleases(ResponseTx<(), settings::Error>, bool),
    /// Set the block_when_disconnected setting.
    SetBlockWhenDisconnected(ResponseTx<(), settings::Error>, bool),
    /// Set the auto-connect setting.
    SetAutoConnect(ResponseTx<(), settings::Error>, bool),
    /// Set the mssfix argument for OpenVPN
    SetOpenVpnMssfix(ResponseTx<(), settings::Error>, Option<u16>),
    /// Set proxy details for OpenVPN
    SetBridgeSettings(ResponseTx<(), settings::Error>, BridgeSettings),
    /// Set proxy state
    SetBridgeState(ResponseTx<(), settings::Error>, BridgeState),
    /// Set if IPv6 should be enabled in the tunnel
    SetEnableIpv6(ResponseTx<(), settings::Error>, bool),
    /// Set DNS options or servers to use
    SetDnsOptions(ResponseTx<(), settings::Error>, DnsOptions),
    /// Toggle macOS network check leak
    /// Set MTU for wireguard tunnels
    SetWireguardMtu(ResponseTx<(), settings::Error>, Option<u16>),
    /// Set automatic key rotation interval for wireguard tunnels
    SetWireguardRotationInterval(ResponseTx<(), settings::Error>, Option<RotationInterval>),
    /// Get the daemon settings
    GetSettings(oneshot::Sender<Settings>),
    /// Generate new wireguard key
    RotateWireguardKey(ResponseTx<(), Error>),
    /// Return a public key of the currently set wireguard private key, if there is one
    GetWireguardKey(ResponseTx<Option<PublicKey>, Error>),
    /// Get information about the currently running and latest app versions
    GetVersionInfo(oneshot::Sender<Option<AppVersionInfo>>),
    /// Return whether the daemon is performing post-upgrade tasks
    IsPerformingPostUpgrade(oneshot::Sender<bool>),
    /// Get current version of the app
    GetCurrentVersion(oneshot::Sender<AppVersion>),
    /// Remove settings and clear the cache
    #[cfg(not(target_os = "android"))]
    FactoryReset(ResponseTx<(), Error>),
    /// Request list of processes excluded from the tunnel
    #[cfg(target_os = "linux")]
    GetSplitTunnelProcesses(ResponseTx<Vec<i32>, split_tunnel::Error>),
    /// Exclude traffic of a process (PID) from the tunnel
    #[cfg(target_os = "linux")]
    AddSplitTunnelProcess(ResponseTx<(), split_tunnel::Error>, i32),
    /// Remove process (PID) from list of processes excluded from the tunnel
    #[cfg(target_os = "linux")]
    RemoveSplitTunnelProcess(ResponseTx<(), split_tunnel::Error>, i32),
    /// Clear list of processes excluded from the tunnel
    #[cfg(target_os = "linux")]
    ClearSplitTunnelProcesses(ResponseTx<(), split_tunnel::Error>),
    /// Exclude traffic of an application from the tunnel
    #[cfg(windows)]
    AddSplitTunnelApp(ResponseTx<(), Error>, PathBuf),
    /// Remove application from list of apps to exclude from the tunnel
    #[cfg(windows)]
    RemoveSplitTunnelApp(ResponseTx<(), Error>, PathBuf),
    /// Clear list of apps to exclude from the tunnel
    #[cfg(windows)]
    ClearSplitTunnelApps(ResponseTx<(), Error>),
    /// Disable split tunnel
    #[cfg(windows)]
    SetSplitTunnelState(ResponseTx<(), Error>, bool),
    /// Toggle wireguard-nt on or off
    #[cfg(target_os = "windows")]
    UseWireGuardNt(ResponseTx<(), Error>, bool),
    /// Notify the split tunnel monitor that a volume was mounted or dismounted
    #[cfg(target_os = "windows")]
    CheckVolumes(ResponseTx<(), Error>),
    /// Register settings for WireGuard obfuscator
    SetObfuscationSettings(ResponseTx<(), settings::Error>, ObfuscationSettings),
    /// Makes the daemon exit the main loop and quit.
    Shutdown,
    /// Saves the target tunnel state and enters a blocking state. The state is restored
    /// upon restart.
    PrepareRestart,
    #[cfg(target_os = "android")]
    BypassSocket(RawFd, oneshot::Sender<()>),
}

/// All events that can happen in the daemon. Sent from various threads and exposed interfaces.
pub(crate) enum InternalDaemonEvent {
    /// Tunnel has changed state.
    TunnelStateTransition(TunnelStateTransition),
    /// Request from the `MullvadTunnelParametersGenerator` to obtain a new relay.
    GenerateTunnelParameters(
        sync_mpsc::Sender<Result<TunnelParameters, ParameterGenerationError>>,
        u32,
    ),
    /// A command sent to the daemon.
    Command(DaemonCommand),
    /// Daemon shutdown triggered by a signal, ctrl-c or similar.
    TriggerShutdown,
    /// The background job fetching new `AppVersionInfo`s got a new info object.
    NewAppVersionInfo(AppVersionInfo),
    /// Sent when a device is updated in any way (key rotation, login, logout, etc.).
    DeviceEvent(PrivateDeviceEvent),
    /// Handles updates from versions without devices.
    DeviceMigrationEvent(Result<PrivateAccountAndDevice, device::Error>),
    /// The split tunnel paths or state were updated.
    #[cfg(target_os = "windows")]
    ExcludedPathsEvent(ExcludedPathsUpdate, oneshot::Sender<Result<(), Error>>),
}

#[cfg(target_os = "windows")]
pub(crate) enum ExcludedPathsUpdate {
    SetState(bool),
    SetPaths(HashSet<PathBuf>),
}

impl From<TunnelStateTransition> for InternalDaemonEvent {
    fn from(tunnel_state_transition: TunnelStateTransition) -> Self {
        InternalDaemonEvent::TunnelStateTransition(tunnel_state_transition)
    }
}

impl From<DaemonCommand> for InternalDaemonEvent {
    fn from(command: DaemonCommand) -> Self {
        InternalDaemonEvent::Command(command)
    }
}

impl From<AppVersionInfo> for InternalDaemonEvent {
    fn from(command: AppVersionInfo) -> Self {
        InternalDaemonEvent::NewAppVersionInfo(command)
    }
}

impl From<PrivateDeviceEvent> for InternalDaemonEvent {
    fn from(event: PrivateDeviceEvent) -> Self {
        InternalDaemonEvent::DeviceEvent(event)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DaemonExecutionState {
    Running,
    Exiting,
    Finished,
}

impl DaemonExecutionState {
    pub fn shutdown(&mut self, tunnel_state: &TunnelState) {
        use self::DaemonExecutionState::*;

        match self {
            Running => {
                match tunnel_state {
                    TunnelState::Disconnected => mem::replace(self, Finished),
                    _ => mem::replace(self, Exiting),
                };
            }
            Exiting | Finished => {}
        };
    }

    pub fn disconnected(&mut self) {
        use self::DaemonExecutionState::*;

        match self {
            Exiting => {
                let _ = mem::replace(self, Finished);
            }
            Running | Finished => {}
        };
    }

    pub fn is_running(&self) -> bool {
        use self::DaemonExecutionState::*;

        match self {
            Running => true,
            Exiting | Finished => false,
        }
    }
}

pub struct DaemonCommandChannel {
    sender: DaemonCommandSender,
    receiver: mpsc::UnboundedReceiver<InternalDaemonEvent>,
}

impl DaemonCommandChannel {
    pub fn new() -> Self {
        let (untracked_sender, receiver) = mpsc::unbounded();
        let sender = DaemonCommandSender(Arc::new(untracked_sender));

        Self { sender, receiver }
    }

    pub fn sender(&self) -> DaemonCommandSender {
        self.sender.clone()
    }

    fn destructure(
        self,
    ) -> (
        DaemonEventSender,
        mpsc::UnboundedReceiver<InternalDaemonEvent>,
    ) {
        let event_sender = DaemonEventSender::new(Arc::downgrade(&self.sender.0));

        (event_sender, self.receiver)
    }
}

#[derive(Clone)]
pub struct DaemonCommandSender(Arc<mpsc::UnboundedSender<InternalDaemonEvent>>);

impl DaemonCommandSender {
    pub fn send(&self, command: DaemonCommand) -> Result<(), Error> {
        self.0
            .unbounded_send(InternalDaemonEvent::Command(command))
            .map_err(|_| Error::DaemonUnavailable)
    }
}

pub(crate) struct DaemonEventSender<E = InternalDaemonEvent> {
    sender: Weak<mpsc::UnboundedSender<InternalDaemonEvent>>,
    _event: PhantomData<E>,
}

impl<E> Clone for DaemonEventSender<E>
where
    InternalDaemonEvent: From<E>,
{
    fn clone(&self) -> Self {
        DaemonEventSender {
            sender: self.sender.clone(),
            _event: PhantomData,
        }
    }
}

impl DaemonEventSender {
    pub fn new(sender: Weak<mpsc::UnboundedSender<InternalDaemonEvent>>) -> Self {
        DaemonEventSender {
            sender,
            _event: PhantomData,
        }
    }

    pub fn to_specialized_sender<E>(&self) -> DaemonEventSender<E>
    where
        InternalDaemonEvent: From<E>,
    {
        DaemonEventSender {
            sender: self.sender.clone(),
            _event: PhantomData,
        }
    }
}

impl<E> DaemonEventSender<E>
where
    InternalDaemonEvent: From<E>,
{
    pub fn is_closed(&self) -> bool {
        self.sender
            .upgrade()
            .map(|sender| sender.is_closed())
            .unwrap_or(true)
    }
}

impl<E> Sender<E> for DaemonEventSender<E>
where
    InternalDaemonEvent: From<E>,
{
    fn send(&self, event: E) -> Result<(), ()> {
        if let Some(sender) = self.sender.upgrade() {
            sender
                .unbounded_send(InternalDaemonEvent::from(event))
                .map_err(|_| ())
        } else {
            Err(())
        }
    }
}

/// Trait representing something that can broadcast daemon events.
pub trait EventListener {
    /// Notify that the tunnel state changed.
    fn notify_new_state(&self, new_state: TunnelState);

    /// Notify that the settings changed.
    fn notify_settings(&self, settings: Settings);

    /// Notify that the relay list changed.
    fn notify_relay_list(&self, relay_list: RelayList);

    /// Notify that info about the latest available app version changed.
    /// Or some flag about the currently running version is changed.
    fn notify_app_version(&self, app_version_info: AppVersionInfo);

    /// Notify that device changed (login, logout, or key rotation).
    fn notify_device_event(&self, event: DeviceEvent);

    /// Notify that a device was revoked using `RemoveDevice`.
    fn notify_remove_device_event(&self, event: RemoveDeviceEvent);
}

pub struct Daemon<L: EventListener> {
    tunnel_command_tx: Arc<mpsc::UnboundedSender<TunnelCommand>>,
    tunnel_state: TunnelState,
    target_state: PersistentTargetState,
    state: DaemonExecutionState,
    #[cfg(target_os = "linux")]
    exclude_pids: split_tunnel::PidManager,
    rx: mpsc::UnboundedReceiver<InternalDaemonEvent>,
    tx: DaemonEventSender,
    reconnection_job: Option<AbortHandle>,
    event_listener: L,
    migration_complete: migrations::MigrationComplete,
    settings: SettingsPersister,
    account_history: account_history::AccountHistory,
    device_checker: device::TunnelStateChangeHandler,
    account_manager: device::AccountManagerHandle,
    api_runtime: mullvad_api::Runtime,
    api_handle: mullvad_api::rest::MullvadRestHandle,
    version_updater_handle: version_check::VersionUpdaterHandle,
    relay_selector: RelaySelector,
    relay_list_updater: RelayListUpdaterHandle,
    last_generated_relays: Option<LastSelectedRelays>,
    app_version_info: Option<AppVersionInfo>,
    shutdown_tasks: Vec<Pin<Box<dyn Future<Output = ()>>>>,
    tunnel_state_machine_handle: tunnel_state_machine::JoinHandle,
    #[cfg(target_os = "windows")]
    volume_update_tx: mpsc::UnboundedSender<()>,
}

impl<L> Daemon<L>
where
    L: EventListener + Clone + Send + 'static,
{
    pub async fn start(
        log_dir: Option<PathBuf>,
        resource_dir: PathBuf,
        settings_dir: PathBuf,
        cache_dir: PathBuf,
        event_listener: L,
        command_channel: DaemonCommandChannel,
        #[cfg(target_os = "android")] android_context: AndroidContext,
    ) -> Result<Self, Error> {
        #[cfg(target_os = "macos")]
        let exclusion_gid = {
            bump_filehandle_limit();
            exclusion_gid::set_exclusion_gid().map_err(Error::GroupIdError)?
        };

        mullvad_api::proxy::ApiConnectionMode::try_delete_cache(&cache_dir).await;

        let (internal_event_tx, internal_event_rx) = command_channel.destructure();

        let api_runtime = mullvad_api::Runtime::with_cache(
            &cache_dir,
            true,
            #[cfg(target_os = "android")]
            Self::create_bypass_tx(&internal_event_tx),
        )
        .await
        .map_err(Error::InitRpcFactory)?;

        let api_availability = api_runtime.availability_handle();
        api_availability.suspend();

        let endpoint_updater = api::ApiEndpointUpdaterHandle::new();

        let migration_data = migrations::migrate_all(&cache_dir, &settings_dir)
            .await
            .unwrap_or_else(|error| {
                log::error!(
                    "{}",
                    error.display_chain_with_msg("Failed to migrate settings or cache")
                );
                None
            });
        let settings = SettingsPersister::load(&settings_dir).await;

        let initial_selector_config = new_selector_config(&settings);
        let relay_selector = RelaySelector::new(initial_selector_config, &resource_dir, &cache_dir);

        let proxy_provider =
            api::ApiConnectionModeProvider::new(cache_dir.clone(), relay_selector.clone());
        let api_handle = api_runtime
            .mullvad_rest_handle(proxy_provider, endpoint_updater.callback())
            .await;

        let migration_complete = if let Some(migration_data) = migration_data {
            migrations::migrate_device(
                migration_data,
                api_handle.clone(),
                internal_event_tx.clone(),
            )
        } else {
            migrations::MigrationComplete::new(true)
        };

        let account_manager = device::AccountManager::spawn(
            api_handle.clone(),
            api_availability.clone(),
            &settings_dir,
            settings
                .tunnel_options
                .wireguard
                .rotation_interval
                .unwrap_or_default(),
        )
        .await
        .map_err(Error::LoadAccountManager)?;
        account_manager
            .receive_events(internal_event_tx.to_specialized_sender())
            .await
            .map_err(Error::LoadAccountManager)?;
        let data = account_manager
            .data()
            .await
            .map_err(Error::LoadAccountManager)?;

        let account_history = account_history::AccountHistory::new(
            &settings_dir,
            data.as_ref().map(|device| device.account_token.clone()),
        )
        .await
        .map_err(Error::LoadAccountHistory)?;

        let target_state = if settings.auto_connect {
            log::info!("Automatically connecting since auto-connect is turned on");
            PersistentTargetState::force(&cache_dir, TargetState::Secured).await
        } else {
            PersistentTargetState::new(&cache_dir).await
        };

        #[cfg(windows)]
        let exclude_paths = if settings.split_tunnel.enable_exclusions {
            settings
                .split_tunnel
                .apps
                .iter()
                .map(|s| OsString::from(s))
                .collect()
        } else {
            vec![]
        };

        let initial_api_endpoint =
            api::get_allowed_endpoint(api_runtime.address_cache.get_address().await);
        let tunnel_parameters_generator = MullvadTunnelParametersGenerator {
            tx: internal_event_tx.clone(),
        };
        let (offline_state_tx, offline_state_rx) = mpsc::unbounded();
        #[cfg(target_os = "windows")]
        let (volume_update_tx, volume_update_rx) = mpsc::unbounded();
        let (tunnel_command_tx, tunnel_state_machine_handle) = tunnel_state_machine::spawn(
            tunnel_state_machine::InitialTunnelState {
                allow_lan: settings.allow_lan,
                block_when_disconnected: settings.block_when_disconnected,
                dns_servers: dns::addresses_from_options(&settings.tunnel_options.dns_options),
                allowed_endpoint: initial_api_endpoint,
                reset_firewall: *target_state != TargetState::Secured,
                #[cfg(windows)]
                exclude_paths,
            },
            tunnel_parameters_generator,
            log_dir,
            resource_dir.clone(),
            internal_event_tx.to_specialized_sender(),
            offline_state_tx,
            #[cfg(target_os = "windows")]
            volume_update_rx,
            #[cfg(target_os = "macos")]
            exclusion_gid,
            #[cfg(target_os = "android")]
            android_context,
        )
        .await
        .map_err(Error::TunnelError)?;

        endpoint_updater.set_tunnel_command_tx(Arc::downgrade(&tunnel_command_tx));

        Self::forward_offline_state(api_availability.clone(), offline_state_rx).await;

        let relay_list_listener = event_listener.clone();
        let on_relay_list_update = move |relay_list: &RelayList| {
            relay_list_listener.notify_relay_list(relay_list.clone());
        };

        let mut relay_list_updater = RelayListUpdater::new(
            relay_selector.clone(),
            api_handle.clone(),
            &cache_dir,
            on_relay_list_update,
        );

        let app_version_info = version_check::load_cache(&cache_dir).await;
        let (version_updater, version_updater_handle) = version_check::VersionUpdater::new(
            api_handle.clone(),
            api_availability.clone(),
            cache_dir.clone(),
            internal_event_tx.to_specialized_sender(),
            app_version_info.clone(),
            settings.show_beta_releases,
        );
        tokio::spawn(version_updater.run());

        // Attempt to download a fresh relay list
        relay_list_updater.update().await;

        let daemon = Daemon {
            tunnel_command_tx,
            tunnel_state: TunnelState::Disconnected,
            target_state,
            state: DaemonExecutionState::Running,
            #[cfg(target_os = "linux")]
            exclude_pids: split_tunnel::PidManager::new().map_err(Error::InitSplitTunneling)?,
            rx: internal_event_rx,
            tx: internal_event_tx,
            reconnection_job: None,
            event_listener,
            migration_complete,
            settings,
            account_history,
            device_checker: device::TunnelStateChangeHandler::new(account_manager.clone()),
            account_manager,
            api_runtime,
            api_handle,
            version_updater_handle,
            relay_selector,
            relay_list_updater,
            last_generated_relays: None,
            app_version_info,
            shutdown_tasks: vec![],
            tunnel_state_machine_handle,
            #[cfg(target_os = "windows")]
            volume_update_tx,
        };

        api_availability.unsuspend();

        Ok(daemon)
    }

    /// Consume the `Daemon` and run the main event loop. Blocks until an error happens or a
    /// shutdown event is received.
    pub async fn run(mut self) -> Result<(), Error> {
        if *self.target_state == TargetState::Secured {
            self.connect_tunnel();
        }

        while let Some(event) = self.rx.next().await {
            self.handle_event(event).await;
            if self.state == DaemonExecutionState::Finished {
                break;
            }
        }

        // If auto-connect is enabled, block all traffic before shutting down to ensure
        // that no traffic can leak during boot.
        #[cfg(windows)]
        if self.settings.auto_connect {
            self.send_tunnel_command(TunnelCommand::BlockWhenDisconnected(true));
        }

        self.finalize().await;
        Ok(())
    }

    async fn finalize(self) {
        let (event_listener, shutdown_tasks, api_runtime, tunnel_state_machine_handle) =
            self.shutdown();
        for future in shutdown_tasks {
            future.await;
        }

        tunnel_state_machine_handle.try_join().await;

        drop(event_listener);
        drop(api_runtime);

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Err(err) = fs::remove_file(mullvad_paths::get_rpc_socket_path()).await {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::error!("Failed to remove old RPC socket: {}", err);
            }
        }
    }

    /// Shuts down the daemon without shutting down the underlying event listener and the shutdown
    /// callbacks
    fn shutdown(
        self,
    ) -> (
        L,
        Vec<Pin<Box<dyn Future<Output = ()>>>>,
        mullvad_api::Runtime,
        tunnel_state_machine::JoinHandle,
    ) {
        let Daemon {
            event_listener,
            mut shutdown_tasks,
            api_runtime,
            tunnel_state_machine_handle,
            target_state,
            account_manager,
            ..
        } = self;

        shutdown_tasks.push(Box::pin(target_state.finalize()));
        shutdown_tasks.push(Box::pin(account_manager.shutdown()));

        (
            event_listener,
            shutdown_tasks,
            api_runtime,
            tunnel_state_machine_handle,
        )
    }

    async fn handle_event(&mut self, event: InternalDaemonEvent) {
        use self::InternalDaemonEvent::*;
        match event {
            TunnelStateTransition(transition) => {
                self.handle_tunnel_state_transition(transition).await
            }
            GenerateTunnelParameters(tunnel_parameters_tx, retry_attempt) => {
                self.handle_generate_tunnel_parameters(&tunnel_parameters_tx, retry_attempt)
                    .await
            }
            Command(command) => self.handle_command(command).await,
            TriggerShutdown => self.trigger_shutdown_event(),
            NewAppVersionInfo(app_version_info) => {
                self.handle_new_app_version_info(app_version_info)
            }
            DeviceEvent(event) => self.handle_device_event(event).await,
            DeviceMigrationEvent(event) => self.handle_device_migration_event(event).await,
            #[cfg(windows)]
            ExcludedPathsEvent(update, tx) => self.handle_new_excluded_paths(update, tx).await,
        }
    }

    async fn handle_tunnel_state_transition(
        &mut self,
        tunnel_state_transition: TunnelStateTransition,
    ) {
        self.reset_rpc_sockets_on_tunnel_state_transition(&tunnel_state_transition)
            .await;
        self.device_checker
            .handle_state_transition(&tunnel_state_transition);

        let tunnel_state = match tunnel_state_transition {
            TunnelStateTransition::Disconnected => TunnelState::Disconnected,
            TunnelStateTransition::Connecting(endpoint) => TunnelState::Connecting {
                endpoint,
                location: self.build_location_from_relay(),
            },
            TunnelStateTransition::Connected(endpoint) => TunnelState::Connected {
                endpoint,
                location: self.build_location_from_relay(),
            },
            TunnelStateTransition::Disconnecting(after_disconnect) => {
                TunnelState::Disconnecting(after_disconnect)
            }
            TunnelStateTransition::Error(error_state) => TunnelState::Error(error_state),
        };

        if !tunnel_state.is_connected() {
            // Cancel reconnects except when entering the connected state.
            // Exempt the latter because a reconnect scheduled while connecting should not be
            // aborted.
            self.unschedule_reconnect();
        }

        log::debug!("New tunnel state: {:?}", tunnel_state);

        match tunnel_state {
            TunnelState::Disconnected => {
                self.api_handle.availability.reset_inactivity_timer();
            }
            _ => {
                self.api_handle.availability.stop_inactivity_timer();
            }
        }

        match tunnel_state {
            TunnelState::Disconnected => self.state.disconnected(),
            TunnelState::Error(ref error_state) => {
                if error_state.is_blocking() {
                    log::info!(
                        "Blocking all network connections, reason: {}",
                        error_state.cause()
                    );
                } else {
                    log::error!(
                        "FAILED TO BLOCK NETWORK CONNECTIONS, ENTERED ERROR STATE BECAUSE: {}",
                        error_state.cause()
                    );
                }

                if let ErrorStateCause::AuthFailed(_) = error_state.cause() {
                    self.schedule_reconnect(Duration::from_secs(60))
                }
            }
            _ => {}
        }

        self.tunnel_state = tunnel_state.clone();
        self.event_listener.notify_new_state(tunnel_state);
    }

    async fn reset_rpc_sockets_on_tunnel_state_transition(
        &mut self,
        tunnel_state_transition: &TunnelStateTransition,
    ) {
        match (&self.tunnel_state, &tunnel_state_transition) {
            // only reset the API sockets if when connected or leaving the connected state
            (&TunnelState::Connected { .. }, _) | (_, &TunnelStateTransition::Connected(_)) => {
                self.api_handle.service().reset();
            }
            _ => (),
        };
    }

    async fn handle_generate_tunnel_parameters(
        &mut self,
        tunnel_parameters_tx: &sync_mpsc::Sender<
            Result<TunnelParameters, ParameterGenerationError>,
        >,
        retry_attempt: u32,
    ) {
        let data = match self.account_manager.data().await {
            Ok(Some(data)) => data,
            _ => {
                log::error!("No account token configured");
                return;
            }
        };

        let result = match self.relay_selector.get_relay(retry_attempt) {
            Ok((SelectedRelay::Custom(custom_relay), _bridge, _obfsucator)) => {
                custom_relay
                    // TODO(emilsp): generate proxy settings for custom tunnels
                    .to_tunnel_parameters(self.settings.tunnel_options.clone(), None)
                    .map_err(|e| {
                        log::error!("Failed to resolve hostname for custom tunnel config: {}", e);
                        ParameterGenerationError::CustomTunnelHostResultionError
                    })
            }
            Ok((SelectedRelay::Normal(constraints), bridge, obfuscator)) => {
                let result = self
                    .create_tunnel_parameters(
                        &constraints.exit_relay,
                        &constraints.entry_relay,
                        constraints.endpoint,
                        bridge,
                        obfuscator,
                        data,
                    )
                    .await;
                result.map_err(|error| match error {
                    Error::NoKeyAvailable => ParameterGenerationError::NoWireguardKey,
                    Error::NoBridgeAvailable => ParameterGenerationError::NoMatchingBridgeRelay,
                    error => {
                        log::error!(
                            "{}",
                            error.display_chain_with_msg("Failed to generate tunnel parameters")
                        );
                        ParameterGenerationError::NoMatchingRelay
                    }
                })
            }
            Err(mullvad_relay_selector::Error::NoBridge) => {
                Err(ParameterGenerationError::NoMatchingBridgeRelay)
            }
            Err(_error) => Err(ParameterGenerationError::NoMatchingRelay),
        };
        if tunnel_parameters_tx.send(result).is_err() {
            log::error!("Failed to send tunnel parameters");
        }
    }

    #[cfg_attr(target_os = "android", allow(unused_variables))]
    async fn create_tunnel_parameters(
        &mut self,
        relay: &Relay,
        entry_relay: &Option<Relay>,
        endpoint: MullvadEndpoint,
        bridge: Option<SelectedBridge>,
        obfuscator: Option<SelectedObfuscator>,
        device: PrivateAccountAndDevice,
    ) -> Result<TunnelParameters, Error> {
        let tunnel_options = self.settings.tunnel_options.clone();
        match endpoint {
            #[cfg(not(target_os = "android"))]
            MullvadEndpoint::OpenVpn(endpoint) => {
                let (bridge_settings, bridge_relay) = match bridge {
                    Some(SelectedBridge::Normal(bridge)) => {
                        (Some(bridge.settings), Some(bridge.relay))
                    }
                    Some(SelectedBridge::Custom(settings)) => (Some(settings), None),
                    None => (None, None),
                };

                self.last_generated_relays = Some(LastSelectedRelays::OpenVpn {
                    relay: relay.clone(),
                    bridge: bridge_relay,
                });

                Ok(openvpn::TunnelParameters {
                    config: openvpn::ConnectionConfig::new(
                        endpoint,
                        device.account_token,
                        "-".to_string(),
                    ),
                    options: tunnel_options.openvpn,
                    generic_options: tunnel_options.generic,
                    proxy: bridge_settings,
                }
                .into())
            }
            #[cfg(target_os = "android")]
            MullvadEndpoint::OpenVpn(endpoint) => {
                unreachable!("OpenVPN is not supported on Android");
            }
            MullvadEndpoint::Wireguard(endpoint) => {
                let tunnel = wireguard::TunnelConfig {
                    private_key: device.device.wg_data.private_key,
                    addresses: vec![
                        device.device.wg_data.addresses.ipv4_address.ip().into(),
                        device.device.wg_data.addresses.ipv6_address.ip().into(),
                    ],
                };

                let (obfuscator_relay, obfuscator_config) = match obfuscator {
                    Some(obfuscator) => (Some(obfuscator.relay), Some(obfuscator.config)),
                    None => (None, None),
                };

                self.last_generated_relays = Some(LastSelectedRelays::WireGuard {
                    wg_entry: entry_relay.clone(),
                    wg_exit: relay.clone(),
                    obfuscator: obfuscator_relay,
                });

                Ok(wireguard::TunnelParameters {
                    connection: wireguard::ConnectionConfig {
                        tunnel,
                        peer: endpoint.peer,
                        exit_peer: endpoint.exit_peer,
                        ipv4_gateway: endpoint.ipv4_gateway,
                        ipv6_gateway: Some(endpoint.ipv6_gateway),
                    },
                    options: tunnel_options.wireguard.options,
                    generic_options: tunnel_options.generic,
                    obfuscation: obfuscator_config,
                }
                .into())
            }
        }
    }

    fn schedule_reconnect(&mut self, delay: Duration) {
        self.unschedule_reconnect();

        let tunnel_command_tx = self.tx.to_specialized_sender();
        let (future, abort_handle) = abortable(Box::pin(async move {
            tokio::time::sleep(delay).await;
            log::debug!("Attempting to reconnect");
            let (tx, rx) = oneshot::channel();
            let _ = tunnel_command_tx.send(DaemonCommand::Reconnect(tx));
            // suppress "unable to send" warning:
            let _ = rx.await;
        }));

        tokio::spawn(future);
        self.reconnection_job = Some(abort_handle);
    }

    fn unschedule_reconnect(&mut self) {
        if let Some(job) = self.reconnection_job.take() {
            job.abort();
        }
    }

    async fn handle_command(&mut self, command: DaemonCommand) {
        use self::DaemonCommand::*;
        if !self.state.is_running() {
            log::trace!("Dropping daemon command because the daemon is shutting down",);
            return;
        }

        if self.tunnel_state.is_disconnected() {
            self.api_handle.availability.reset_inactivity_timer();
        }

        match command {
            SetTargetState(tx, state) => self.on_set_target_state(tx, state).await,
            Reconnect(tx) => self.on_reconnect(tx),
            GetState(tx) => self.on_get_state(tx),
            GetCurrentLocation(tx) => self.on_get_current_location(tx).await,
            CreateNewAccount(tx) => self.on_create_new_account(tx).await,
            GetAccountData(tx, account_token) => self.on_get_account_data(tx, account_token).await,
            GetWwwAuthToken(tx) => self.on_get_www_auth_token(tx).await,
            SubmitVoucher(tx, voucher) => self.on_submit_voucher(tx, voucher).await,
            GetRelayLocations(tx) => self.on_get_relay_locations(tx),
            UpdateRelayLocations => self.on_update_relay_locations().await,
            LoginAccount(tx, account_token) => self.on_login_account(tx, account_token),
            LogoutAccount(tx) => self.on_logout_account(tx),
            GetDevice(tx) => self.on_get_device(tx).await,
            UpdateDevice(tx) => self.on_update_device(tx).await,
            ListDevices(tx, account_token) => self.on_list_devices(tx, account_token).await,
            RemoveDevice(tx, account_token, device_id) => {
                self.on_remove_device(tx, account_token, device_id).await
            }
            GetAccountHistory(tx) => self.on_get_account_history(tx),
            ClearAccountHistory(tx) => self.on_clear_account_history(tx).await,
            UpdateRelaySettings(tx, update) => self.on_update_relay_settings(tx, update).await,
            SetAllowLan(tx, allow_lan) => self.on_set_allow_lan(tx, allow_lan).await,
            SetShowBetaReleases(tx, enabled) => self.on_set_show_beta_releases(tx, enabled).await,
            SetBlockWhenDisconnected(tx, block_when_disconnected) => {
                self.on_set_block_when_disconnected(tx, block_when_disconnected)
                    .await
            }
            SetAutoConnect(tx, auto_connect) => self.on_set_auto_connect(tx, auto_connect).await,
            SetOpenVpnMssfix(tx, mssfix_arg) => self.on_set_openvpn_mssfix(tx, mssfix_arg).await,
            SetBridgeSettings(tx, bridge_settings) => {
                self.on_set_bridge_settings(tx, bridge_settings).await
            }
            SetBridgeState(tx, bridge_state) => self.on_set_bridge_state(tx, bridge_state).await,
            SetEnableIpv6(tx, enable_ipv6) => self.on_set_enable_ipv6(tx, enable_ipv6).await,
            SetDnsOptions(tx, dns_servers) => self.on_set_dns_options(tx, dns_servers).await,
            SetWireguardMtu(tx, mtu) => self.on_set_wireguard_mtu(tx, mtu).await,
            SetWireguardRotationInterval(tx, interval) => {
                self.on_set_wireguard_rotation_interval(tx, interval).await
            }
            GetSettings(tx) => self.on_get_settings(tx),
            RotateWireguardKey(tx) => self.on_rotate_wireguard_key(tx).await,
            GetWireguardKey(tx) => self.on_get_wireguard_key(tx).await,
            GetVersionInfo(tx) => self.on_get_version_info(tx).await,
            IsPerformingPostUpgrade(tx) => self.on_is_performing_post_upgrade(tx).await,
            GetCurrentVersion(tx) => self.on_get_current_version(tx),
            #[cfg(not(target_os = "android"))]
            FactoryReset(tx) => self.on_factory_reset(tx).await,
            #[cfg(target_os = "linux")]
            GetSplitTunnelProcesses(tx) => self.on_get_split_tunnel_processes(tx),
            #[cfg(target_os = "linux")]
            AddSplitTunnelProcess(tx, pid) => self.on_add_split_tunnel_process(tx, pid),
            #[cfg(target_os = "linux")]
            RemoveSplitTunnelProcess(tx, pid) => self.on_remove_split_tunnel_process(tx, pid),
            #[cfg(target_os = "linux")]
            ClearSplitTunnelProcesses(tx) => self.on_clear_split_tunnel_processes(tx),
            #[cfg(windows)]
            AddSplitTunnelApp(tx, path) => self.on_add_split_tunnel_app(tx, path).await,
            #[cfg(windows)]
            RemoveSplitTunnelApp(tx, path) => self.on_remove_split_tunnel_app(tx, path).await,
            #[cfg(windows)]
            ClearSplitTunnelApps(tx) => self.on_clear_split_tunnel_apps(tx).await,
            #[cfg(windows)]
            SetSplitTunnelState(tx, enabled) => self.on_set_split_tunnel_state(tx, enabled).await,
            #[cfg(target_os = "windows")]
            UseWireGuardNt(tx, state) => self.on_use_wireguard_nt(tx, state).await,
            #[cfg(target_os = "windows")]
            CheckVolumes(tx) => self.on_check_volumes(tx).await,
            SetObfuscationSettings(tx, settings) => {
                self.on_set_obfuscation_settings(tx, settings).await
            }
            Shutdown => self.trigger_shutdown_event(),
            PrepareRestart => self.on_prepare_restart(),
            #[cfg(target_os = "android")]
            BypassSocket(fd, tx) => self.on_bypass_socket(fd, tx),
        }
    }

    fn handle_new_app_version_info(&mut self, app_version_info: AppVersionInfo) {
        self.app_version_info = Some(app_version_info.clone());
        self.event_listener.notify_app_version(app_version_info);
    }

    async fn handle_device_event(&mut self, event: PrivateDeviceEvent) {
        match &event {
            PrivateDeviceEvent::Login(device) => {
                if let Err(error) = self.account_history.set(device.account_token.clone()).await {
                    log::error!(
                        "{}",
                        error.display_chain_with_msg("Failed to update account history")
                    );
                }
                if *self.target_state == TargetState::Secured {
                    log::debug!("Initiating tunnel restart because the account token changed");
                    self.reconnect_tunnel();
                }
            }
            PrivateDeviceEvent::Logout => {
                log::info!("Disconnecting because account token was cleared");
                self.set_target_state(TargetState::Unsecured).await;
            }
            PrivateDeviceEvent::Revoked => {
                // If we're currently in a secured state, reconnect to make sure we immediately
                // enter the error state.
                if *self.target_state == TargetState::Secured {
                    self.connect_tunnel();
                }
            }
            PrivateDeviceEvent::RotatedKey(_) => {
                if let Some(TunnelType::Wireguard) = self.get_target_tunnel_type() {
                    self.schedule_reconnect(WG_RECONNECT_DELAY);
                }
            }
            _ => (),
        }
        self.event_listener
            .notify_device_event(DeviceEvent::from(event));
    }

    async fn handle_device_migration_event(
        &mut self,
        result: Result<PrivateAccountAndDevice, device::Error>,
    ) {
        let account_manager = self.account_manager.clone();
        let event_listener = self.event_listener.clone();
        tokio::spawn(async move {
            if let Ok(Some(_)) = account_manager.data_after_login().await {
                // Discard stale device
                return;
            }

            let result = async { account_manager.set(result?).await }.await;

            if let Err(error) = result {
                log::error!(
                    "{}",
                    error.display_chain_with_msg("Failed to move over account from old settings")
                );
                // Synthesize a logout event.
                event_listener.notify_device_event(DeviceEvent::revoke(false));
            }
        });
    }

    #[cfg(windows)]
    async fn handle_new_excluded_paths(
        &mut self,
        update: ExcludedPathsUpdate,
        tx: ResponseTx<(), Error>,
    ) {
        let save_result = match update {
            ExcludedPathsUpdate::SetState(state) => self
                .settings
                .set_split_tunnel_state(state)
                .await
                .map_err(Error::SettingsError),
            ExcludedPathsUpdate::SetPaths(paths) => self
                .settings
                .set_split_tunnel_apps(paths)
                .await
                .map_err(Error::SettingsError),
        };
        let changed = *save_result.as_ref().unwrap_or(&false);
        let _ = tx.send(save_result.map(|_| ()));
        if changed {
            self.event_listener
                .notify_settings(self.settings.to_settings());
        }
    }

    async fn on_set_target_state(
        &mut self,
        tx: oneshot::Sender<bool>,
        new_target_state: TargetState,
    ) {
        if self.state.is_running() {
            let state_change_initated = self.set_target_state(new_target_state).await;
            Self::oneshot_send(tx, state_change_initated, "state change initiated");
        } else {
            log::warn!("Ignoring target state change request due to shutdown");
        }
    }

    fn on_reconnect(&mut self, tx: oneshot::Sender<bool>) {
        if *self.target_state == TargetState::Secured || self.tunnel_state.is_in_error_state() {
            self.connect_tunnel();
            Self::oneshot_send(tx, true, "reconnect issued");
        } else {
            log::debug!("Ignoring reconnect command. Currently not in secured state");
            Self::oneshot_send(tx, false, "reconnect issued");
        }
    }

    fn on_get_state(&self, tx: oneshot::Sender<TunnelState>) {
        Self::oneshot_send(tx, self.tunnel_state.clone(), "current state");
    }

    async fn on_is_performing_post_upgrade(&self, tx: oneshot::Sender<bool>) {
        let performing_post_upgrade = !self.migration_complete.is_complete();
        Self::oneshot_send(tx, performing_post_upgrade, "performing post upgrade");
    }

    async fn on_get_current_location(&mut self, tx: oneshot::Sender<Option<GeoIpLocation>>) {
        use self::TunnelState::*;

        match &self.tunnel_state {
            Disconnected => {
                let location = self.get_geo_location().await;
                tokio::spawn(async {
                    Self::oneshot_send(tx, location.await.ok(), "current location");
                });
            }
            Connecting { location, .. } => {
                Self::oneshot_send(tx, location.clone(), "current location")
            }
            Disconnecting(..) => {
                Self::oneshot_send(tx, self.build_location_from_relay(), "current location")
            }
            Connected { location, .. } => {
                let relay_location = location.clone();
                let location_future = self.get_geo_location().await;
                tokio::spawn(async {
                    let location = location_future.await;
                    Self::oneshot_send(
                        tx,
                        location.ok().map(|fetched_location| GeoIpLocation {
                            ipv4: fetched_location.ipv4,
                            ipv6: fetched_location.ipv6,
                            ..relay_location.unwrap_or(fetched_location)
                        }),
                        "current location",
                    );
                });
            }
            Error(_) => {
                // We are not online at all at this stage so no location data is available.
                Self::oneshot_send(tx, None, "current location");
            }
        }
    }

    async fn get_geo_location(&mut self) -> impl Future<Output = Result<GeoIpLocation, ()>> {
        let rest_service = self.api_runtime.rest_handle().await;
        async {
            geoip::send_location_request(rest_service)
                .await
                .map_err(|e| {
                    log::warn!("Unable to fetch GeoIP location: {}", e.display_chain());
                })
        }
    }

    fn build_location_from_relay(&self) -> Option<GeoIpLocation> {
        let relays = self.last_generated_relays.as_ref()?;
        let hostname;
        let bridge_hostname;
        let entry_hostname;
        let obfuscator_hostname;
        let location;
        let take_hostname =
            |relay: &Option<Relay>| relay.as_ref().map(|relay| relay.hostname.clone());

        match relays {
            LastSelectedRelays::WireGuard {
                wg_entry: entry,
                wg_exit: exit,
                obfuscator,
            } => {
                entry_hostname = take_hostname(entry);
                hostname = exit.hostname.clone();
                obfuscator_hostname = take_hostname(obfuscator);
                bridge_hostname = None;
                location = exit.location.as_ref().cloned().unwrap();
            }
            #[cfg(not(target_os = "android"))]
            LastSelectedRelays::OpenVpn { relay, bridge } => {
                hostname = relay.hostname.clone();
                bridge_hostname = take_hostname(bridge);
                entry_hostname = None;
                obfuscator_hostname = None;
                location = relay.location.as_ref().cloned().unwrap();
            }
        };

        Some(GeoIpLocation {
            ipv4: None,
            ipv6: None,
            country: location.country,
            city: Some(location.city),
            latitude: location.latitude,
            longitude: location.longitude,
            mullvad_exit_ip: true,
            hostname: Some(hostname),
            bridge_hostname,
            entry_hostname,
            obfuscator_hostname,
        })
    }

    async fn on_create_new_account(&mut self, tx: ResponseTx<String, Error>) {
        let account_manager = self.account_manager.clone();
        tokio::spawn(async move {
            let result = async {
                if let Ok(Some(_)) = account_manager.data().await {
                    return Err(Error::AlreadyLoggedIn);
                }
                let token = account_manager
                    .account_service
                    .create_account()
                    .await
                    .map_err(Error::RestError)?;
                account_manager
                    .login(token.clone())
                    .await
                    .map_err(|error| {
                        log::error!(
                            "{}",
                            error.display_chain_with_msg("Creating new account failed")
                        );
                        Error::LoginError(error)
                    })?;
                Ok(token)
            };
            Self::oneshot_send(tx, result.await, "create new account");
        });
    }

    async fn on_get_account_data(
        &mut self,
        tx: ResponseTx<AccountData, mullvad_api::rest::Error>,
        account_token: AccountToken,
    ) {
        let account = self.account_manager.account_service.clone();
        tokio::spawn(async move {
            let result = account.check_expiry(account_token).await;
            Self::oneshot_send(
                tx,
                result.map(|expiry| AccountData { expiry }),
                "account data",
            );
        });
    }

    async fn on_get_www_auth_token(&mut self, tx: ResponseTx<String, Error>) {
        if let Ok(Some(device)) = self.account_manager.data().await {
            let future = self
                .account_manager
                .account_service
                .get_www_auth_token(device.account_token);
            tokio::spawn(async {
                Self::oneshot_send(
                    tx,
                    future.await.map_err(Error::RestError),
                    "get_www_auth_token response",
                );
            });
        } else {
            Self::oneshot_send(
                tx,
                Err(Error::NoAccountToken),
                "get_www_auth_token response",
            );
        }
    }

    async fn on_submit_voucher(
        &mut self,
        tx: ResponseTx<VoucherSubmission, Error>,
        voucher: String,
    ) {
        if let Ok(Some(device)) = self.account_manager.data().await {
            let mut account = self.account_manager.account_service.clone();
            tokio::spawn(async move {
                Self::oneshot_send(
                    tx,
                    account
                        .submit_voucher(device.account_token, voucher)
                        .await
                        .map_err(Error::RestError),
                    "submit_voucher response",
                );
            });
        } else {
            Self::oneshot_send(tx, Err(Error::NoAccountToken), "submit_voucher response");
        }
    }

    fn on_get_relay_locations(&mut self, tx: oneshot::Sender<RelayList>) {
        Self::oneshot_send(tx, self.relay_selector.get_locations(), "relay locations");
    }

    async fn on_update_relay_locations(&mut self) {
        self.relay_list_updater.update().await;
    }

    fn on_login_account(&mut self, tx: ResponseTx<(), Error>, account_token: String) {
        let account_manager = self.account_manager.clone();
        tokio::spawn(async move {
            let result = async {
                account_manager.login(account_token).await.map_err(|error| {
                    log::error!("{}", error.display_chain_with_msg("Login failed"));
                    Error::LoginError(error)
                })
            };
            Self::oneshot_send(tx, result.await, "login_account response");
        });
    }

    fn on_logout_account(&mut self, tx: ResponseTx<(), Error>) {
        let account_manager = self.account_manager.clone();
        tokio::spawn(async move {
            let result = async {
                account_manager.logout().await.map_err(|error| {
                    log::error!("{}", error.display_chain_with_msg("Logout failed"));
                    Error::LogoutError(error)
                })
            };
            Self::oneshot_send(tx, result.await, "logout_account response");
        });
    }

    async fn on_get_device(&mut self, tx: ResponseTx<Option<AccountAndDevice>, Error>) {
        let account_manager = self.account_manager.clone();
        tokio::spawn(async move {
            Self::oneshot_send(
                tx,
                Ok(account_manager
                    .data()
                    .await
                    .unwrap_or(None)
                    .map(AccountAndDevice::from)),
                "get_device response",
            );
        });
    }

    async fn on_update_device(&mut self, tx: ResponseTx<(), Error>) {
        let account_manager = self.account_manager.clone();
        tokio::spawn(async move {
            let result = match account_manager.validate_device().await {
                Ok(_) | Err(device::Error::NoDevice) => Ok(()),
                Err(error) => Err(error),
            };
            Self::oneshot_send(
                tx,
                result.map_err(Error::UpdateDeviceError),
                "update_device response",
            );
        });
    }

    async fn on_list_devices(&self, tx: ResponseTx<Vec<Device>, Error>, token: AccountToken) {
        let service = self.account_manager.device_service.clone();
        tokio::spawn(async move {
            Self::oneshot_send(
                tx,
                service
                    .list_devices(token)
                    .await
                    .map_err(Error::ListDevicesError),
                "list_devices response",
            );
        });
    }

    async fn on_remove_device(
        &mut self,
        tx: ResponseTx<(), Error>,
        token: AccountToken,
        device_id: DeviceId,
    ) {
        let device_service = self.account_manager.device_service.clone();
        let event_listener = self.event_listener.clone();

        tokio::spawn(async move {
            let mut devices = match device_service
                .list_devices(token.clone())
                .await
                .map_err(Error::ListDevicesError)
            {
                Ok(devices) => devices,
                Err(error) => {
                    Self::oneshot_send(tx, Err(error), "remove_device response");
                    return;
                }
            };
            if let Err(error) = device_service
                .remove_device(token.clone(), device_id.clone())
                .await
                .map_err(Error::RemoveDeviceError)
            {
                Self::oneshot_send(tx, Err(error), "remove_device response");
                return;
            };
            let removed_device =
                if let Some(index) = devices.iter().position(|device| device.id == device_id) {
                    devices.swap_remove(index)
                } else {
                    log::error!("List did not contain the revoked device");
                    Device {
                        id: device_id,
                        name: "unknown device".to_string(),
                        pubkey: talpid_types::net::wireguard::PublicKey::from([0u8; 32]),
                        ports: vec![],
                    }
                };
            event_listener.notify_remove_device_event(RemoveDeviceEvent {
                account_token: token,
                removed_device,
                new_devices: devices,
            });
            Self::oneshot_send(tx, Ok(()), "remove_device response");
        });
    }

    fn on_get_account_history(&mut self, tx: oneshot::Sender<Option<AccountToken>>) {
        Self::oneshot_send(
            tx,
            self.account_history.get(),
            "get_account_history response",
        );
    }

    async fn on_clear_account_history(&mut self, tx: ResponseTx<(), Error>) {
        let result = self
            .account_history
            .clear()
            .await
            .map_err(Error::AccountHistory);
        Self::oneshot_send(tx, result, "clear_account_history response");
    }

    async fn on_get_version_info(&mut self, tx: oneshot::Sender<Option<AppVersionInfo>>) {
        if self.app_version_info.is_none() {
            log::debug!("No version cache found. Fetching new info");
            let mut handle = self.version_updater_handle.clone();
            tokio::spawn(async move {
                Self::oneshot_send(
                    tx,
                    handle
                        .run_version_check()
                        .await
                        .map_err(|error| {
                            log::error!(
                                "{}",
                                error.display_chain_with_msg("Error running version check")
                            )
                        })
                        .ok(),
                    "get_version_info response",
                );
            });
        } else {
            Self::oneshot_send(
                tx,
                self.app_version_info.clone(),
                "get_version_info response",
            );
        }
    }

    fn on_get_current_version(&mut self, tx: oneshot::Sender<AppVersion>) {
        Self::oneshot_send(
            tx,
            version::PRODUCT_VERSION.to_owned(),
            "get_current_version response",
        );
    }

    #[cfg(not(target_os = "android"))]
    async fn on_factory_reset(&mut self, tx: ResponseTx<(), Error>) {
        let mut last_error = Ok(());

        if let Err(error) = self.account_manager.logout().await {
            log::error!(
                "{}",
                error.display_chain_with_msg("Failed to clear device cache")
            );
            last_error = Err(Error::LogoutError(error));
        }

        if let Err(error) = self.account_history.clear().await {
            log::error!(
                "{}",
                error.display_chain_with_msg("Failed to clear account history")
            );
            last_error = Err(Error::ClearAccountHistoryError(error));
        }

        if let Err(e) = self.settings.reset().await {
            log::error!("Failed to reset settings: {}", e);
            last_error = Err(Error::ClearSettingsError(e));
        }

        // Shut the daemon down.
        self.trigger_shutdown_event();

        self.shutdown_tasks.push(Box::pin(async move {
            if let Err(e) = Self::clear_cache_directory().await {
                log::error!(
                    "{}",
                    e.display_chain_with_msg("Failed to clear cache directory")
                );
                last_error = Err(Error::ClearCacheError);
            }

            if let Err(e) = Self::clear_log_directory().await {
                log::error!(
                    "{}",
                    e.display_chain_with_msg("Failed to clear log directory")
                );
                last_error = Err(Error::ClearLogsError);
            }
            Self::oneshot_send(tx, last_error, "factory_reset response");
        }));
    }

    #[cfg(target_os = "linux")]
    fn on_get_split_tunnel_processes(&mut self, tx: ResponseTx<Vec<i32>, split_tunnel::Error>) {
        let result = self.exclude_pids.list().map_err(|error| {
            log::error!("{}", error.display_chain_with_msg("Unable to obtain PIDs"));
            error
        });
        Self::oneshot_send(tx, result, "get_split_tunnel_processes response");
    }

    #[cfg(target_os = "linux")]
    fn on_add_split_tunnel_process(&mut self, tx: ResponseTx<(), split_tunnel::Error>, pid: i32) {
        let result = self.exclude_pids.add(pid).map_err(|error| {
            log::error!("{}", error.display_chain_with_msg("Unable to add PID"));
            error
        });
        Self::oneshot_send(tx, result, "add_split_tunnel_process response");
    }

    #[cfg(target_os = "linux")]
    fn on_remove_split_tunnel_process(
        &mut self,
        tx: ResponseTx<(), split_tunnel::Error>,
        pid: i32,
    ) {
        let result = self.exclude_pids.remove(pid).map_err(|error| {
            log::error!("{}", error.display_chain_with_msg("Unable to remove PID"));
            error
        });
        Self::oneshot_send(tx, result, "remove_split_tunnel_process response");
    }

    #[cfg(target_os = "linux")]
    fn on_clear_split_tunnel_processes(&mut self, tx: ResponseTx<(), split_tunnel::Error>) {
        let result = self.exclude_pids.clear().map_err(|error| {
            log::error!("{}", error.display_chain_with_msg("Unable to clear PIDs"));
            error
        });
        Self::oneshot_send(tx, result, "clear_split_tunnel_processes response");
    }

    /// Update the split app paths in both the settings and tunnel
    #[cfg(windows)]
    async fn set_split_tunnel_paths(
        &mut self,
        tx: ResponseTx<(), Error>,
        response_msg: &'static str,
        settings: Settings,
        update: ExcludedPathsUpdate,
    ) {
        let new_list = match update {
            ExcludedPathsUpdate::SetPaths(ref paths) => {
                if *paths == settings.split_tunnel.apps {
                    Self::oneshot_send(tx, Ok(()), response_msg);
                    return;
                }
                paths.iter()
            }
            ExcludedPathsUpdate::SetState(_) => settings.split_tunnel.apps.iter(),
        };
        let new_state = match update {
            ExcludedPathsUpdate::SetPaths(_) => settings.split_tunnel.enable_exclusions,
            ExcludedPathsUpdate::SetState(state) => {
                if state == settings.split_tunnel.enable_exclusions {
                    Self::oneshot_send(tx, Ok(()), response_msg);
                    return;
                }
                state
            }
        };

        if new_state || new_state != settings.split_tunnel.enable_exclusions {
            let tunnel_list = if new_state {
                new_list.map(|s| OsString::from(s)).collect()
            } else {
                vec![]
            };

            let (result_tx, result_rx) = oneshot::channel();
            self.send_tunnel_command(TunnelCommand::SetExcludedApps(result_tx, tunnel_list));
            let daemon_tx = self.tx.clone();

            tokio::spawn(async move {
                match result_rx.await {
                    Ok(Ok(_)) => (),
                    Ok(Err(error)) => {
                        log::error!(
                            "{}",
                            error.display_chain_with_msg("Failed to set excluded apps list")
                        );
                        Self::oneshot_send(tx, Err(Error::SplitTunnelError(error)), response_msg);
                        return;
                    }
                    Err(_) => {
                        log::error!("The tunnel failed to return a result");
                        return;
                    }
                }

                let _ = daemon_tx.send(InternalDaemonEvent::ExcludedPathsEvent(update, tx));
            });
        } else {
            let _ = self
                .tx
                .send(InternalDaemonEvent::ExcludedPathsEvent(update, tx));
        }
    }

    #[cfg(windows)]
    async fn on_add_split_tunnel_app(&mut self, tx: ResponseTx<(), Error>, path: PathBuf) {
        let settings = self.settings.to_settings();

        let mut new_list = settings.split_tunnel.apps.clone();
        new_list.insert(path);

        self.set_split_tunnel_paths(
            tx,
            "add_split_tunnel_app response",
            settings,
            ExcludedPathsUpdate::SetPaths(new_list),
        )
        .await;
    }

    #[cfg(windows)]
    async fn on_remove_split_tunnel_app(&mut self, tx: ResponseTx<(), Error>, path: PathBuf) {
        let settings = self.settings.to_settings();

        let mut new_list = settings.split_tunnel.apps.clone();
        new_list.remove(&path);

        self.set_split_tunnel_paths(
            tx,
            "remove_split_tunnel_app response",
            settings,
            ExcludedPathsUpdate::SetPaths(new_list),
        )
        .await;
    }

    #[cfg(windows)]
    async fn on_clear_split_tunnel_apps(&mut self, tx: ResponseTx<(), Error>) {
        let settings = self.settings.to_settings();
        let new_list = HashSet::new();
        self.set_split_tunnel_paths(
            tx,
            "clear_split_tunnel_apps response",
            settings,
            ExcludedPathsUpdate::SetPaths(new_list),
        )
        .await;
    }

    #[cfg(windows)]
    async fn on_set_split_tunnel_state(&mut self, tx: ResponseTx<(), Error>, state: bool) {
        let settings = self.settings.to_settings();
        self.set_split_tunnel_paths(
            tx,
            "set_split_tunnel_state response",
            settings,
            ExcludedPathsUpdate::SetState(state),
        )
        .await;
    }

    #[cfg(windows)]
    async fn on_use_wireguard_nt(&mut self, tx: ResponseTx<(), Error>, state: bool) {
        let save_result = self
            .settings
            .set_use_wireguard_nt(state)
            .await
            .map_err(Error::SettingsError);
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "use_wireguard_nt response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    if let Some(TunnelType::Wireguard) = self.get_connected_tunnel_type() {
                        log::info!("Initiating tunnel restart");
                        self.reconnect_tunnel();
                    }
                }
            }
            Err(error) => {
                log::error!(
                    "{}",
                    error.display_chain_with_msg("Unable to save settings")
                );
                Self::oneshot_send(tx, Err(error), "use_wireguard_nt response");
            }
        }
    }

    #[cfg(windows)]
    async fn on_check_volumes(&mut self, tx: ResponseTx<(), Error>) {
        if self.volume_update_tx.unbounded_send(()).is_ok() {
            let _ = tx.send(Ok(()));
        }
    }

    async fn on_update_relay_settings(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        update: RelaySettingsUpdate,
    ) {
        let save_result = self.settings.update_relay_settings(update).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "update_relay_settings response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    self.relay_selector
                        .set_config(new_selector_config(&self.settings));
                    log::info!("Initiating tunnel restart because the relay settings changed");
                    self.reconnect_tunnel();
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "update_relay_settings response");
            }
        }
    }

    async fn on_set_allow_lan(&mut self, tx: ResponseTx<(), settings::Error>, allow_lan: bool) {
        let save_result = self.settings.set_allow_lan(allow_lan).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_allow_lan response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    self.send_tunnel_command(TunnelCommand::AllowLan(allow_lan));
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_allow_lan response");
            }
        }
    }

    async fn on_set_show_beta_releases(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        enabled: bool,
    ) {
        let save_result = self.settings.set_show_beta_releases(enabled).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_show_beta_releases response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    let mut handle = self.version_updater_handle.clone();
                    handle.set_show_beta_releases(enabled).await;
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_show_beta_releases response");
            }
        }
    }

    async fn on_set_block_when_disconnected(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        block_when_disconnected: bool,
    ) {
        let save_result = self
            .settings
            .set_block_when_disconnected(block_when_disconnected)
            .await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_block_when_disconnected response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    self.send_tunnel_command(TunnelCommand::BlockWhenDisconnected(
                        block_when_disconnected,
                    ));
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_block_when_disconnected response");
            }
        }
    }

    async fn on_set_auto_connect(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        auto_connect: bool,
    ) {
        let save_result = self.settings.set_auto_connect(auto_connect).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set auto-connect response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set auto-connect response");
            }
        }
    }

    async fn on_set_openvpn_mssfix(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        mssfix_arg: Option<u16>,
    ) {
        let save_result = self.settings.set_openvpn_mssfix(mssfix_arg).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_openvpn_mssfix response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    if let Some(TunnelType::OpenVpn) = self.get_connected_tunnel_type() {
                        log::info!(
                            "Initiating tunnel restart because the OpenVPN mssfix setting changed"
                        );
                        self.reconnect_tunnel();
                    }
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_openvpn_mssfix response");
            }
        }
    }

    async fn on_set_bridge_settings(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        new_settings: BridgeSettings,
    ) {
        match self.settings.set_bridge_settings(new_settings).await {
            Ok(settings_changes) => {
                if settings_changes {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    self.relay_selector
                        .set_config(new_selector_config(&self.settings));
                    if let Err(error) = self.api_handle.service().next_api_endpoint().await {
                        log::error!("Failed to rotate API endpoint: {}", error);
                    }
                    self.reconnect_tunnel();
                };
                Self::oneshot_send(tx, Ok(()), "set_bridge_settings");
            }

            Err(e) => {
                log::error!(
                    "{}",
                    e.display_chain_with_msg("Failed to set new bridge settings")
                );
                Self::oneshot_send(tx, Err(e), "set_bridge_settings");
            }
        }
    }

    async fn on_set_obfuscation_settings(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        new_settings: ObfuscationSettings,
    ) {
        match self.settings.set_obfuscation_settings(new_settings).await {
            Ok(settings_changed) => {
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    self.relay_selector
                        .set_config(new_selector_config(&self.settings));
                    self.reconnect_tunnel();
                }
                Self::oneshot_send(tx, Ok(()), "set_obfuscation_settings");
            }
            Err(err) => {
                log::error!(
                    "{}",
                    err.display_chain_with_msg("Failed to set obfuscation settings")
                );
                Self::oneshot_send(tx, Err(err), "set_obfuscation_settings");
            }
        }
    }

    async fn on_set_bridge_state(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        bridge_state: BridgeState,
    ) {
        let result = match self.settings.set_bridge_state(bridge_state).await {
            Ok(settings_changed) => {
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    self.relay_selector
                        .set_config(new_selector_config(&self.settings));
                    log::info!("Initiating tunnel restart because bridge state changed");
                    self.reconnect_tunnel();
                }
                Ok(())
            }
            Err(error) => {
                log::error!(
                    "{}",
                    error.display_chain_with_msg("Failed to set new bridge state")
                );
                Err(error)
            }
        };
        Self::oneshot_send(tx, result, "on_set_bridge_state response");
    }

    async fn on_set_enable_ipv6(&mut self, tx: ResponseTx<(), settings::Error>, enable_ipv6: bool) {
        let save_result = self.settings.set_enable_ipv6(enable_ipv6).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_enable_ipv6 response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    log::info!("Initiating tunnel restart because the enable IPv6 setting changed");
                    self.reconnect_tunnel();
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_enable_ipv6 response");
            }
        }
    }

    async fn on_set_dns_options(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        dns_options: DnsOptions,
    ) {
        let save_result = self.settings.set_dns_options(dns_options.clone()).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_dns_options response");
                if settings_changed {
                    let settings = self.settings.to_settings();
                    let resolvers =
                        dns::addresses_from_options(&settings.tunnel_options.dns_options);
                    self.event_listener.notify_settings(settings);
                    self.send_tunnel_command(TunnelCommand::Dns(resolvers));
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_dns_options response");
            }
        }
    }

    async fn on_set_wireguard_mtu(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        mtu: Option<u16>,
    ) {
        let save_result = self.settings.set_wireguard_mtu(mtu).await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_wireguard_mtu response");
                if settings_changed {
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                    if let Some(TunnelType::Wireguard) = self.get_connected_tunnel_type() {
                        log::info!(
                            "Initiating tunnel restart because the WireGuard MTU setting changed"
                        );
                        self.reconnect_tunnel();
                    }
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_wireguard_mtu response");
            }
        }
    }

    async fn on_set_wireguard_rotation_interval(
        &mut self,
        tx: ResponseTx<(), settings::Error>,
        interval: Option<RotationInterval>,
    ) {
        let save_result = self
            .settings
            .set_wireguard_rotation_interval(interval)
            .await;
        match save_result {
            Ok(settings_changed) => {
                Self::oneshot_send(tx, Ok(()), "set_wireguard_rotation_interval response");
                if settings_changed {
                    if let Err(error) = self
                        .account_manager
                        .set_rotation_interval(interval.unwrap_or_default())
                        .await
                    {
                        log::error!(
                            "{}",
                            error.display_chain_with_msg("Failed to update rotation interval")
                        );
                    }
                    self.event_listener
                        .notify_settings(self.settings.to_settings());
                }
            }
            Err(e) => {
                log::error!("{}", e.display_chain_with_msg("Unable to save settings"));
                Self::oneshot_send(tx, Err(e), "set_wireguard_rotation_interval response");
            }
        }
    }

    async fn on_rotate_wireguard_key(&self, tx: ResponseTx<(), Error>) {
        let manager = self.account_manager.clone();
        tokio::spawn(async move {
            let result = manager
                .rotate_key()
                .await
                .map(|_| ())
                .map_err(Error::KeyRotationError);
            Self::oneshot_send(tx, result, "rotate_wireguard_key response");
        });
    }

    async fn on_get_wireguard_key(&self, tx: ResponseTx<Option<PublicKey>, Error>) {
        let result = if let Ok(Some(config)) = self.account_manager.data().await {
            Ok(Some(config.device.wg_data.get_public_key()))
        } else {
            Err(Error::NoAccountToken)
        };
        Self::oneshot_send(tx, result, "get_wireguard_key response");
    }

    fn on_get_settings(&self, tx: oneshot::Sender<Settings>) {
        Self::oneshot_send(tx, self.settings.to_settings(), "get_settings response");
    }

    fn oneshot_send<T>(tx: oneshot::Sender<T>, t: T, msg: &'static str) {
        if tx.send(t).is_err() {
            log::warn!("Unable to send {} to the daemon command sender", msg);
        }
    }

    fn trigger_shutdown_event(&mut self) {
        self.state.shutdown(&self.tunnel_state);
        self.disconnect_tunnel();
    }

    fn on_prepare_restart(&mut self) {
        // TODO: See if this can be made to also shut down the daemon
        //       without causing the service to be restarted.

        if *self.target_state == TargetState::Secured {
            self.send_tunnel_command(TunnelCommand::BlockWhenDisconnected(true));
        }
        self.target_state.lock();
    }

    #[cfg(target_os = "android")]
    fn on_bypass_socket(&mut self, fd: RawFd, tx: oneshot::Sender<()>) {
        match self.tunnel_state {
            // When connected, the API connection shouldn't be bypassed.
            TunnelState::Connected { .. } => (),
            _ => {
                self.send_tunnel_command(TunnelCommand::BypassSocket(fd, tx));
            }
        }
    }

    #[cfg(target_os = "android")]
    fn create_bypass_tx(
        event_sender: &DaemonEventSender,
    ) -> Option<mpsc::Sender<mullvad_api::SocketBypassRequest>> {
        let (bypass_tx, mut bypass_rx) = mpsc::channel(1);
        let daemon_tx = event_sender.to_specialized_sender();
        tokio::spawn(async move {
            while let Some((raw_fd, done_tx)) = bypass_rx.next().await {
                if let Err(_) = daemon_tx.send(DaemonCommand::BypassSocket(raw_fd, done_tx)) {
                    log::error!("Can't send socket bypass request to daemon");
                    break;
                }
            }
        });
        Some(bypass_tx)
    }

    async fn forward_offline_state(
        api_availability: ApiAvailabilityHandle,
        mut offline_state_rx: mpsc::UnboundedReceiver<bool>,
    ) {
        let initial_state = offline_state_rx
            .next()
            .await
            .expect("missing initial offline state");
        api_availability.set_offline(initial_state);
        tokio::spawn(async move {
            while let Some(is_offline) = offline_state_rx.next().await {
                api_availability.set_offline(is_offline);
            }
        });
    }

    /// Set the target state of the client. If it changed trigger the operations needed to
    /// progress towards that state.
    /// Returns a bool representing whether or not a state change was initiated.
    async fn set_target_state(&mut self, new_state: TargetState) -> bool {
        if new_state != *self.target_state || self.tunnel_state.is_in_error_state() {
            log::debug!("Target state {:?} => {:?}", *self.target_state, new_state);

            self.target_state.set(new_state).await;

            match *self.target_state {
                TargetState::Secured => self.connect_tunnel(),
                TargetState::Unsecured => self.disconnect_tunnel(),
            }
            true
        } else {
            false
        }
    }

    fn connect_tunnel(&mut self) {
        self.api_runtime.availability_handle().resume_background();
        self.send_tunnel_command(TunnelCommand::Connect);
    }

    fn disconnect_tunnel(&mut self) {
        self.send_tunnel_command(TunnelCommand::Disconnect);
    }

    fn reconnect_tunnel(&mut self) {
        if *self.target_state == TargetState::Secured {
            self.connect_tunnel();
        }
    }

    fn get_connected_tunnel_type(&self) -> Option<TunnelType> {
        if let TunnelState::Connected {
            endpoint: TunnelEndpoint { tunnel_type, .. },
            ..
        } = self.tunnel_state
        {
            Some(tunnel_type)
        } else {
            None
        }
    }

    fn get_target_tunnel_type(&self) -> Option<TunnelType> {
        match self.tunnel_state {
            TunnelState::Connected {
                endpoint: TunnelEndpoint { tunnel_type, .. },
                ..
            }
            | TunnelState::Connecting {
                endpoint: TunnelEndpoint { tunnel_type, .. },
                ..
            } => Some(tunnel_type),
            _ => None,
        }
    }

    fn send_tunnel_command(&mut self, command: TunnelCommand) {
        self.tunnel_command_tx
            .unbounded_send(command)
            .expect("Tunnel state machine has stopped");
    }

    #[cfg(not(target_os = "android"))]
    async fn clear_log_directory() -> Result<(), Error> {
        let log_dir = mullvad_paths::get_log_dir().map_err(Error::PathError)?;
        Self::clear_directory(&log_dir).await
    }

    #[cfg(not(target_os = "android"))]
    async fn clear_cache_directory() -> Result<(), Error> {
        let cache_dir = mullvad_paths::cache_dir().map_err(Error::PathError)?;
        Self::clear_directory(&cache_dir).await
    }

    #[cfg(not(target_os = "android"))]
    async fn clear_directory(path: &Path) -> Result<(), Error> {
        #[cfg(not(target_os = "windows"))]
        {
            fs::remove_dir_all(path)
                .await
                .map_err(|e| Error::RemoveDirError(path.display().to_string(), e))?;
            fs::create_dir_all(path)
                .await
                .map_err(|e| Error::CreateDirError(path.display().to_string(), e))
        }
        #[cfg(target_os = "windows")]
        {
            let mut dir = fs::read_dir(&path).await.map_err(Error::ReadDirError)?;

            let mut result = Ok(());

            while let Some(entry) = dir.next_entry().await.map_err(Error::FileEntryError)? {
                let entry_type = match entry.file_type().await {
                    Ok(entry_type) => entry_type,
                    Err(error) => {
                        result = result.and(Err(Error::FileTypeError(error)));
                        continue;
                    }
                };

                let removal = if entry_type.is_file() || entry_type.is_symlink() {
                    fs::remove_file(entry.path()).await
                } else {
                    fs::remove_dir_all(entry.path()).await
                };
                result = result.and(
                    removal
                        .map_err(|e| Error::RemoveDirError(entry.path().display().to_string(), e)),
                );
            }
            result
        }
    }

    pub fn shutdown_handle(&self) -> DaemonShutdownHandle {
        DaemonShutdownHandle {
            tx: self.tx.clone(),
        }
    }
}

pub struct DaemonShutdownHandle {
    tx: DaemonEventSender,
}

impl DaemonShutdownHandle {
    pub fn shutdown(&self) {
        let _ = self.tx.send(InternalDaemonEvent::TriggerShutdown);
    }
}

struct MullvadTunnelParametersGenerator {
    tx: DaemonEventSender,
}

impl TunnelParametersGenerator for MullvadTunnelParametersGenerator {
    fn generate(
        &mut self,
        retry_attempt: u32,
    ) -> Result<TunnelParameters, ParameterGenerationError> {
        let (response_tx, response_rx) = sync_mpsc::channel();
        if self
            .tx
            .send(InternalDaemonEvent::GenerateTunnelParameters(
                response_tx,
                retry_attempt,
            ))
            .is_err()
        {
            log::error!("Failed to send daemon command to generate tunnel parameters!");
            return Err(ParameterGenerationError::NoMatchingRelay);
        }

        match response_rx.recv() {
            Ok(result) => result,
            Err(_) => {
                log::error!("Failed to receive tunnel parameter generation result!");
                Err(ParameterGenerationError::NoMatchingRelay)
            }
        }
    }
}

/// Contains all relays that were selected last time when tunnel parameters were generated.
enum LastSelectedRelays {
    /// Represents all relays generated for a WireGuard tunnel.
    /// The traffic flow can look like this:
    ///     client -> obfuscator -> entry -> exit -> internet
    /// But for most users, it will look like this:
    ///     client -> entry -> internet
    WireGuard {
        wg_entry: Option<Relay>,
        wg_exit: Relay,
        obfuscator: Option<Relay>,
    },
    /// Represents all relays generated for an OpenVPN tunnel.
    /// The traffic flows like this:
    ///     client -> bridge -> relay -> internet
    #[cfg(not(target_os = "android"))]
    OpenVpn { relay: Relay, bridge: Option<Relay> },
}

fn new_selector_config(settings: &Settings) -> SelectorConfig {
    SelectorConfig {
        relay_settings: settings.get_relay_settings(),
        bridge_state: settings.get_bridge_state(),
        bridge_settings: settings.bridge_settings.clone(),
        obfuscation_settings: settings.obfuscation_settings.clone(),
    }
}

/// Bump filehandle limit
#[cfg(target_os = "macos")]
pub fn bump_filehandle_limit() {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `&mut limits` is a valid pointer parameter for the getrlimit syscall
    let status = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) };
    if status != 0 {
        log::error!(
            "Failed to get file handle limits: {}-{}",
            io::Error::from_raw_os_error(status),
            status
        );
        return;
    }

    const INCREASED_FILEHANDLE_LIMIT: u64 = 1024;
    // if file handle limit is already big enough, there's no reason to decrease it.
    if limits.rlim_cur >= INCREASED_FILEHANDLE_LIMIT {
        return;
    }

    limits.rlim_cur = INCREASED_FILEHANDLE_LIMIT;
    // SAFETY: `&limits` is a valid pointer parameter for the getrlimit syscall
    let status = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) };
    if status != 0 {
        log::error!(
            "Failed to set file handle limit to {}: {}-{}",
            INCREASED_FILEHANDLE_LIMIT,
            io::Error::from_raw_os_error(status),
            status
        );
    }
}
