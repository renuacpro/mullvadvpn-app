package net.mullvad.mullvadvpn.service

import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import net.mullvad.mullvadvpn.model.AccountAndDevice
import net.mullvad.mullvadvpn.model.AppVersionInfo
import net.mullvad.mullvadvpn.model.Device
import net.mullvad.mullvadvpn.model.DeviceEvent
import net.mullvad.mullvadvpn.model.DeviceState
import net.mullvad.mullvadvpn.model.DnsOptions
import net.mullvad.mullvadvpn.model.GeoIpLocation
import net.mullvad.mullvadvpn.model.GetAccountDataResult
import net.mullvad.mullvadvpn.model.KeygenEvent
import net.mullvad.mullvadvpn.model.LoginResult
import net.mullvad.mullvadvpn.model.PublicKey
import net.mullvad.mullvadvpn.model.RelayList
import net.mullvad.mullvadvpn.model.RelaySettingsUpdate
import net.mullvad.mullvadvpn.model.RemoveDeviceEvent
import net.mullvad.mullvadvpn.model.RemoveDeviceResult
import net.mullvad.mullvadvpn.model.Settings
import net.mullvad.mullvadvpn.model.TunnelState
import net.mullvad.mullvadvpn.model.VoucherSubmissionResult
import net.mullvad.talpid.util.EventNotifier

class MullvadDaemon(vpnService: MullvadVpnService) {
    protected var daemonInterfaceAddress = 0L

    var onDeviceRemoved = EventNotifier<RemoveDeviceEvent?>(null)
    val onSettingsChange = EventNotifier<Settings?>(null)
    var onTunnelStateChange = EventNotifier<TunnelState>(TunnelState.Disconnected)

    var onAppVersionInfoChange: ((AppVersionInfo) -> Unit)? = null
    var onKeygenEvent: ((KeygenEvent) -> Unit)? = null
    var onRelayListChange: ((RelayList) -> Unit)? = null
    var onDaemonStopped: (() -> Unit)? = null

    private val _deviceStateUpdates = MutableSharedFlow<DeviceState>(
        extraBufferCapacity = 1,
        onBufferOverflow = BufferOverflow.DROP_OLDEST
    )
    val deviceStateUpdates = _deviceStateUpdates.asSharedFlow()

    init {
        System.loadLibrary("mullvad_jni")
        initialize(vpnService, vpnService.cacheDir.absolutePath, vpnService.filesDir.absolutePath)

        onSettingsChange.notify(getSettings())

        onTunnelStateChange.notify(getState() ?: TunnelState.Disconnected)
    }

    fun connect() {
        connect(daemonInterfaceAddress)
    }

    fun createNewAccount(): String? {
        return createNewAccount(daemonInterfaceAddress)
    }

    fun disconnect() {
        disconnect(daemonInterfaceAddress)
    }

    fun generateWireguardKey(): KeygenEvent? {
        // TODO: remove
        return null
    }

    fun getAccountData(accountToken: String): GetAccountDataResult {
        return getAccountData(daemonInterfaceAddress, accountToken)
    }

    fun getAccountHistory(): String? {
        return getAccountHistory(daemonInterfaceAddress)
    }

    fun getWwwAuthToken(): String {
        return getWwwAuthToken(daemonInterfaceAddress) ?: ""
    }

    fun getCurrentLocation(): GeoIpLocation? {
        return getCurrentLocation(daemonInterfaceAddress)
    }

    fun getCurrentVersion(): String? {
        return getCurrentVersion(daemonInterfaceAddress)
    }

    fun getRelayLocations(): RelayList? {
        return getRelayLocations(daemonInterfaceAddress)
    }

    fun getSettings(): Settings? {
        return getSettings(daemonInterfaceAddress)
    }

    fun getState(): TunnelState? {
        return getState(daemonInterfaceAddress)
    }

    fun getVersionInfo(): AppVersionInfo? {
        return getVersionInfo(daemonInterfaceAddress)
    }

    fun getWireguardKey(): PublicKey? {
        // TODO: no longer needed
        return getWireguardKey(daemonInterfaceAddress)
    }

    fun reconnect() {
        reconnect(daemonInterfaceAddress)
    }

    fun clearAccountHistory() {
        clearAccountHistory(daemonInterfaceAddress)
    }

    fun loginAccount(accountToken: String): LoginResult {
        return loginAccount(daemonInterfaceAddress, accountToken)
    }

    fun logoutAccount() = logoutAccount(daemonInterfaceAddress)

    fun listDevices(accountToken: String?): List<Device>? {
        return listDevices(daemonInterfaceAddress, accountToken)
    }

    fun getDevice(): AccountAndDevice? = getDevice(daemonInterfaceAddress)

    fun updateDevice() = updateDevice(daemonInterfaceAddress)

    fun removeDevice(accountToken: String, deviceId: String): RemoveDeviceResult {
        return removeDevice(daemonInterfaceAddress, accountToken, deviceId)
    }

    fun setAllowLan(allowLan: Boolean) {
        setAllowLan(daemonInterfaceAddress, allowLan)
    }

    fun setAutoConnect(autoConnect: Boolean) {
        setAutoConnect(daemonInterfaceAddress, autoConnect)
    }

    fun setDnsOptions(dnsOptions: DnsOptions) {
        setDnsOptions(daemonInterfaceAddress, dnsOptions)
    }

    fun setWireguardMtu(wireguardMtu: Int?) {
        setWireguardMtu(daemonInterfaceAddress, wireguardMtu)
    }

    fun shutdown() {
        shutdown(daemonInterfaceAddress)
    }

    fun submitVoucher(voucher: String): VoucherSubmissionResult {
        return submitVoucher(daemonInterfaceAddress, voucher)
    }

    fun updateRelaySettings(update: RelaySettingsUpdate) {
        updateRelaySettings(daemonInterfaceAddress, update)
    }

    fun verifyWireguardKey(): Boolean? {
        return verifyWireguardKey(daemonInterfaceAddress)
    }

    fun onDestroy() {
        onSettingsChange.unsubscribeAll()
        onTunnelStateChange.unsubscribeAll()

        onAppVersionInfoChange = null
        onKeygenEvent = null
        onRelayListChange = null
        onDaemonStopped = null

        deinitialize()
    }

    private external fun initialize(
        vpnService: MullvadVpnService,
        cacheDirectory: String,
        resourceDirectory: String
    )

    private external fun deinitialize()

    private external fun connect(daemonInterfaceAddress: Long)
    private external fun createNewAccount(daemonInterfaceAddress: Long): String?
    private external fun disconnect(daemonInterfaceAddress: Long)
    private external fun getAccountData(
        daemonInterfaceAddress: Long,
        accountToken: String
    ): GetAccountDataResult

    private external fun getAccountHistory(daemonInterfaceAddress: Long): String?
    private external fun getWwwAuthToken(daemonInterfaceAddress: Long): String?
    private external fun getCurrentLocation(daemonInterfaceAddress: Long): GeoIpLocation?
    private external fun getCurrentVersion(daemonInterfaceAddress: Long): String?
    private external fun getRelayLocations(daemonInterfaceAddress: Long): RelayList?
    private external fun getSettings(daemonInterfaceAddress: Long): Settings?
    private external fun getState(daemonInterfaceAddress: Long): TunnelState?
    private external fun getVersionInfo(daemonInterfaceAddress: Long): AppVersionInfo?
    private external fun getWireguardKey(daemonInterfaceAddress: Long): PublicKey?
    private external fun reconnect(daemonInterfaceAddress: Long)
    private external fun clearAccountHistory(daemonInterfaceAddress: Long)
    private external fun loginAccount(
        daemonInterfaceAddress: Long,
        accountToken: String?
    ): LoginResult

    private external fun logoutAccount(daemonInterfaceAddress: Long)
    private external fun listDevices(
        daemonInterfaceAddress: Long,
        accountToken: String?
    ): List<Device>?

    private external fun getDevice(daemonInterfaceAddress: Long): AccountAndDevice?
    private external fun updateDevice(daemonInterfaceAddress: Long)
    private external fun removeDevice(
        daemonInterfaceAddress: Long,
        accountToken: String?,
        deviceId: String
    ): RemoveDeviceResult

    private external fun setAllowLan(daemonInterfaceAddress: Long, allowLan: Boolean)
    private external fun setAutoConnect(daemonInterfaceAddress: Long, alwaysOn: Boolean)
    private external fun setDnsOptions(daemonInterfaceAddress: Long, dnsOptions: DnsOptions)
    private external fun setWireguardMtu(daemonInterfaceAddress: Long, wireguardMtu: Int?)
    private external fun shutdown(daemonInterfaceAddress: Long)
    private external fun submitVoucher(
        daemonInterfaceAddress: Long,
        voucher: String
    ): VoucherSubmissionResult

    private external fun updateRelaySettings(
        daemonInterfaceAddress: Long,
        update: RelaySettingsUpdate
    )

    private external fun verifyWireguardKey(daemonInterfaceAddress: Long): Boolean?

    private fun notifyAppVersionInfoEvent(appVersionInfo: AppVersionInfo) {
        onAppVersionInfoChange?.invoke(appVersionInfo)
    }

    private fun notifyRelayListEvent(relayList: RelayList) {
        onRelayListChange?.invoke(relayList)
    }

    private fun notifySettingsEvent(settings: Settings) {
        onSettingsChange.notify(settings)
    }

    private fun notifyTunnelStateEvent(event: TunnelState) {
        onTunnelStateChange.notify(event)
    }

    private fun notifyDaemonStopped() {
        onDaemonStopped?.invoke()
    }

    private fun notifyDeviceEvent(event: DeviceEvent) {
        _deviceStateUpdates.tryEmit(DeviceState.from(event.device))
    }

    private fun notifyRemoveDeviceEvent(event: RemoveDeviceEvent) {
        onDeviceRemoved.notify(event)
    }
}
