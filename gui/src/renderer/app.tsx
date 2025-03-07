import { batch, Provider } from 'react-redux';
import { Router } from 'react-router';
import { bindActionCreators } from 'redux';

import { ILinuxSplitTunnelingApplication, IWindowsApplication } from '../shared/application-types';
import {
  AccountToken,
  BridgeSettings,
  BridgeState,
  IAccountData,
  IAppVersionInfo,
  IDevice,
  IDeviceConfig,
  IDeviceEvent,
  IDeviceRemoval,
  IDnsOptions,
  ILocation,
  ISettings,
  liftConstraint,
  RelaySettings,
  RelaySettingsUpdate,
  TunnelState,
  VoucherResponse,
} from '../shared/daemon-rpc-types';
import { messages, relayLocations } from '../shared/gettext';
import { IGuiSettingsState, SYSTEM_PREFERRED_LOCALE_KEY } from '../shared/gui-settings-state';
import { IRelayListPair, LaunchApplicationResult } from '../shared/ipc-schema';
import {
  IChangelog,
  ICurrentAppVersionInfo,
  IHistoryObject,
  ScrollPositions,
} from '../shared/ipc-types';
import log, { ConsoleOutput } from '../shared/logging';
import { LogLevel } from '../shared/logging-types';
import { Scheduler } from '../shared/scheduler';
import AppRouter from './components/AppRouter';
import { Changelog } from './components/Changelog';
import ErrorBoundary from './components/ErrorBoundary';
import MacOsScrollbarDetection from './components/MacOsScrollbarDetection';
import { ModalContainer } from './components/Modal';
import PlatformWindowContainer from './containers/PlatformWindowContainer';
import { AppContext } from './context';
import History, { ITransitionSpecification, transitions } from './lib/history';
import { loadTranslations } from './lib/load-translations';
import IpcOutput from './lib/logging';
import { RoutePath } from './lib/routes';
import accountActions from './redux/account/actions';
import connectionActions from './redux/connection/actions';
import settingsActions from './redux/settings/actions';
import configureStore from './redux/store';
import userInterfaceActions from './redux/userinterface/actions';
import versionActions from './redux/version/actions';

const IpcRendererEventChannel = window.ipc;

interface IPreferredLocaleDescriptor {
  name: string;
  code: string;
}

type LoginState = 'none' | 'logging in' | 'creating account' | 'too many devices';

const SUPPORTED_LOCALE_LIST = [
  { name: 'Dansk', code: 'da' },
  { name: 'Deutsch', code: 'de' },
  { name: 'English', code: 'en' },
  { name: 'Español', code: 'es' },
  { name: 'Suomi', code: 'fi' },
  { name: 'Français', code: 'fr' },
  { name: 'Italiano', code: 'it' },
  { name: '日本語', code: 'ja' },
  { name: '한국어', code: 'ko' },
  { name: 'မြန်မာဘာသာ', code: 'my' },
  { name: 'Nederlands', code: 'nl' },
  { name: 'Norsk', code: 'nb' },
  { name: 'Język polski', code: 'pl' },
  { name: 'Português', code: 'pt' },
  { name: 'Русский', code: 'ru' },
  { name: 'Svenska', code: 'sv' },
  { name: 'ภาษาไทย', code: 'th' },
  { name: 'Türkçe', code: 'tr' },
  { name: '简体中文', code: 'zh-CN' },
  { name: '繁體中文', code: 'zh-TW' },
];

export default class AppRenderer {
  private history: History;
  private reduxStore = configureStore();
  private reduxActions = {
    account: bindActionCreators(accountActions, this.reduxStore.dispatch),
    connection: bindActionCreators(connectionActions, this.reduxStore.dispatch),
    settings: bindActionCreators(settingsActions, this.reduxStore.dispatch),
    version: bindActionCreators(versionActions, this.reduxStore.dispatch),
    userInterface: bindActionCreators(userInterfaceActions, this.reduxStore.dispatch),
  };

  private location?: Partial<ILocation>;
  private lastDisconnectedLocation?: Partial<ILocation>;
  private relayListPair!: IRelayListPair;
  private tunnelState!: TunnelState;
  private settings!: ISettings;
  private deviceConfig?: IDeviceConfig;
  private hasReceivedDeviceConfig = false;
  private guiSettings!: IGuiSettingsState;
  private loginState: LoginState = 'none';
  private previousLoginState: LoginState = 'none';
  private loginScheduler = new Scheduler();
  private connectedToDaemon = false;
  private getLocationPromise?: Promise<ILocation>;

  constructor() {
    log.addOutput(new ConsoleOutput(LogLevel.debug));
    log.addOutput(new IpcOutput(LogLevel.debug));

    IpcRendererEventChannel.window.listenShape((windowShapeParams) => {
      if (typeof windowShapeParams.arrowPosition === 'number') {
        this.reduxActions.userInterface.updateWindowArrowPosition(windowShapeParams.arrowPosition);
      }
    });

    IpcRendererEventChannel.daemon.listenConnected(() => {
      void this.onDaemonConnected();
    });

    IpcRendererEventChannel.daemon.listenDisconnected(() => {
      this.onDaemonDisconnected();
    });

    IpcRendererEventChannel.daemon.listenIsPerformingPostUpgrade((isPerformingPostUpgrade) => {
      this.setIsPerformingPostUpgrade(isPerformingPostUpgrade);
    });

    IpcRendererEventChannel.account.listen((newAccountData?: IAccountData) => {
      this.setAccountExpiry(newAccountData?.expiry);
    });

    IpcRendererEventChannel.account.listenDevice((deviceEvent) => {
      const oldDeviceConfig = this.deviceConfig;
      this.hasReceivedDeviceConfig = true;
      this.handleAccountChange(deviceEvent, oldDeviceConfig?.accountToken);
    });

    IpcRendererEventChannel.account.listenDevices((devices) => {
      this.reduxActions.account.updateDevices(devices);
    });

    IpcRendererEventChannel.accountHistory.listen((newAccountHistory?: AccountToken) => {
      this.setAccountHistory(newAccountHistory);
    });

    IpcRendererEventChannel.tunnel.listen((newState: TunnelState) => {
      this.setTunnelState(newState);
      this.updateBlockedState(newState, this.settings.blockWhenDisconnected);
    });

    IpcRendererEventChannel.settings.listen((newSettings: ISettings) => {
      this.setSettings(newSettings);
      this.updateBlockedState(this.tunnelState, newSettings.blockWhenDisconnected);
    });

    IpcRendererEventChannel.relays.listen((relayListPair: IRelayListPair) => {
      this.setRelayListPair(relayListPair);
    });

    IpcRendererEventChannel.currentVersion.listen((currentVersion: ICurrentAppVersionInfo) => {
      this.setCurrentVersion(currentVersion);
    });

    IpcRendererEventChannel.upgradeVersion.listen((upgradeVersion: IAppVersionInfo) => {
      this.setUpgradeVersion(upgradeVersion);
    });

    IpcRendererEventChannel.guiSettings.listen((guiSettings: IGuiSettingsState) => {
      this.setGuiSettings(guiSettings);
    });

    IpcRendererEventChannel.autoStart.listen((autoStart: boolean) => {
      this.storeAutoStart(autoStart);
    });

    IpcRendererEventChannel.windowsSplitTunneling.listen((applications: IWindowsApplication[]) => {
      this.reduxActions.settings.setSplitTunnelingApplications(applications);
    });

    IpcRendererEventChannel.window.listenFocus((focus: boolean) => {
      this.reduxActions.userInterface.setWindowFocused(focus);
    });

    IpcRendererEventChannel.window.listenMacOsScrollbarVisibility((visibility) => {
      this.reduxActions.userInterface.setMacOsScrollbarVisibility(visibility);
    });

    IpcRendererEventChannel.navigation.listenReset(() =>
      this.history.dismiss(true, transitions.none),
    );

    // Request the initial state from the main process
    const initialState = IpcRendererEventChannel.state.get();

    this.setLocale(initialState.translations.locale);
    loadTranslations(
      messages,
      initialState.translations.locale,
      initialState.translations.messages,
    );
    loadTranslations(
      relayLocations,
      initialState.translations.locale,
      initialState.translations.relayLocations,
    );

    this.setAccountExpiry(initialState.accountData?.expiry);
    this.setSettings(initialState.settings);
    this.setIsPerformingPostUpgrade(initialState.isPerformingPostUpgrade);
    this.handleAccountChange(
      { deviceConfig: initialState.deviceConfig },
      undefined,
      initialState.navigationHistory !== undefined,
    );
    this.hasReceivedDeviceConfig = initialState.hasReceivedDeviceConfig;
    this.setAccountHistory(initialState.accountHistory);
    this.setTunnelState(initialState.tunnelState);
    this.updateBlockedState(initialState.tunnelState, initialState.settings.blockWhenDisconnected);

    this.setRelayListPair(initialState.relayListPair);
    this.setCurrentVersion(initialState.currentVersion);
    this.setUpgradeVersion(initialState.upgradeVersion);
    this.setGuiSettings(initialState.guiSettings);
    this.storeAutoStart(initialState.autoStart);
    this.setChangelog(initialState.changelog);

    if (initialState.macOsScrollbarVisibility !== undefined) {
      this.reduxActions.userInterface.setMacOsScrollbarVisibility(
        initialState.macOsScrollbarVisibility,
      );
    }

    if (initialState.isConnected) {
      void this.onDaemonConnected();
    }

    this.checkContentHeight(false);
    window.addEventListener('resize', () => {
      this.checkContentHeight(true);
    });

    if (initialState.windowsSplitTunnelingApplications) {
      this.reduxActions.settings.setSplitTunnelingApplications(
        initialState.windowsSplitTunnelingApplications,
      );
    }

    void this.updateLocation();

    this.reduxActions.userInterface.setScrollPositions(initialState.scrollPositions);

    if (initialState.navigationHistory) {
      // Set last action to POP to trigger automatic scrolling to saved coordinates.
      initialState.navigationHistory.lastAction = 'POP';
      this.history = History.fromSavedHistory(initialState.navigationHistory);
    } else {
      const navigationBase = this.getNavigationBase();
      this.history = new History(navigationBase);
    }
  }

  public renderView() {
    return (
      <AppContext.Provider value={{ app: this }}>
        <Provider store={this.reduxStore}>
          <Router history={this.history.asHistory}>
            <PlatformWindowContainer>
              <ErrorBoundary>
                <ModalContainer>
                  <AppRouter />
                  <Changelog />
                  {window.env.platform === 'darwin' && <MacOsScrollbarDetection />}
                </ModalContainer>
              </ErrorBoundary>
            </PlatformWindowContainer>
          </Router>
        </Provider>
      </AppContext.Provider>
    );
  }

  public login = async (accountToken: AccountToken) => {
    const actions = this.reduxActions;
    actions.account.startLogin(accountToken);

    log.info('Logging in');

    this.previousLoginState = this.loginState;
    this.loginState = 'logging in';

    try {
      await IpcRendererEventChannel.account.login(accountToken);
    } catch (e) {
      const error = e as Error;
      if (error.message === 'Too many devices') {
        actions.account.loginTooManyDevices(error);
        this.loginState = 'too many devices';
        this.history.reset(RoutePath.tooManyDevices, transitions.push);
      } else {
        actions.account.loginFailed(error);
      }
    }
  };

  public cancelLogin = (): void => {
    const reduxAccount = this.reduxActions.account;
    reduxAccount.loggedOut();
    this.loginState = 'none';
  };

  public async logout() {
    try {
      await IpcRendererEventChannel.account.logout();
    } catch (e) {
      const error = e as Error;
      log.info('Failed to logout: ', error.message);
    }
  }

  public leaveRevokedDevice = async () => {
    const reduxAccount = this.reduxActions.account;
    reduxAccount.loggedOut();
    this.resetNavigation();
    await this.disconnectTunnel();
  };

  public async createNewAccount() {
    log.info('Creating account');

    const actions = this.reduxActions;
    actions.account.startCreateAccount();
    this.loginState = 'creating account';

    try {
      await IpcRendererEventChannel.account.create();
      this.redirectToConnect();
    } catch (e) {
      const error = e as Error;
      actions.account.createAccountFailed(error);
    }
  }

  public submitVoucher(voucherCode: string): Promise<VoucherResponse> {
    return IpcRendererEventChannel.account.submitVoucher(voucherCode);
  }

  public updateAccountData(): void {
    IpcRendererEventChannel.account.updateData();
  }

  public getDevice = (): Promise<IDevice | undefined> => {
    return IpcRendererEventChannel.account.getDevice();
  };

  public fetchDevices = async (accountToken: AccountToken): Promise<Array<IDevice>> => {
    const devices = await IpcRendererEventChannel.account.listDevices(accountToken);
    this.reduxActions.account.updateDevices(devices);
    return devices;
  };

  public removeDevice(deviceRemoval: IDeviceRemoval): Promise<void> {
    return IpcRendererEventChannel.account.removeDevice(deviceRemoval);
  }

  public async connectTunnel(): Promise<void> {
    return IpcRendererEventChannel.tunnel.connect();
  }

  public async disconnectTunnel(): Promise<void> {
    return IpcRendererEventChannel.tunnel.disconnect();
  }

  public async reconnectTunnel(): Promise<void> {
    return IpcRendererEventChannel.tunnel.reconnect();
  }

  public updateRelaySettings(relaySettings: RelaySettingsUpdate) {
    return IpcRendererEventChannel.settings.updateRelaySettings(relaySettings);
  }

  public updateBridgeSettings(bridgeSettings: BridgeSettings) {
    return IpcRendererEventChannel.settings.updateBridgeSettings(bridgeSettings);
  }

  public setDnsOptions(dns: IDnsOptions) {
    return IpcRendererEventChannel.settings.setDnsOptions(dns);
  }

  public clearAccountHistory(): Promise<void> {
    return IpcRendererEventChannel.accountHistory.clear();
  }

  public openLinkWithAuth = async (link: string): Promise<void> => {
    let token = '';
    try {
      token = await IpcRendererEventChannel.account.getWwwAuthToken();
    } catch (e) {
      const error = e as Error;
      log.error(`Failed to get the WWW auth token: ${error.message}`);
    }
    void this.openUrl(`${link}?token=${token}`);
  };

  public async setAllowLan(allowLan: boolean) {
    const actions = this.reduxActions;
    await IpcRendererEventChannel.settings.setAllowLan(allowLan);
    actions.settings.updateAllowLan(allowLan);
  }

  public async setShowBetaReleases(showBetaReleases: boolean) {
    const actions = this.reduxActions;
    await IpcRendererEventChannel.settings.setShowBetaReleases(showBetaReleases);
    actions.settings.updateShowBetaReleases(showBetaReleases);
  }

  public async setEnableIpv6(enableIpv6: boolean) {
    const actions = this.reduxActions;
    await IpcRendererEventChannel.settings.setEnableIpv6(enableIpv6);
    actions.settings.updateEnableIpv6(enableIpv6);
  }

  public async setBridgeState(bridgeState: BridgeState) {
    const actions = this.reduxActions;
    await IpcRendererEventChannel.settings.setBridgeState(bridgeState);
    actions.settings.updateBridgeState(bridgeState);
  }

  public setBlockWhenDisconnected = async (blockWhenDisconnected: boolean) => {
    const actions = this.reduxActions;
    await IpcRendererEventChannel.settings.setBlockWhenDisconnected(blockWhenDisconnected);
    actions.settings.updateBlockWhenDisconnected(blockWhenDisconnected);
  };

  public async setOpenVpnMssfix(mssfix?: number) {
    const actions = this.reduxActions;
    actions.settings.updateOpenVpnMssfix(mssfix);
    await IpcRendererEventChannel.settings.setOpenVpnMssfix(mssfix);
  }

  public async setWireguardMtu(mtu?: number) {
    const actions = this.reduxActions;
    actions.settings.updateWireguardMtu(mtu);
    await IpcRendererEventChannel.settings.setWireguardMtu(mtu);
  }

  public setAutoConnect(autoConnect: boolean) {
    IpcRendererEventChannel.guiSettings.setAutoConnect(autoConnect);
  }

  public setEnableSystemNotifications(flag: boolean) {
    IpcRendererEventChannel.guiSettings.setEnableSystemNotifications(flag);
  }

  public setAutoStart(autoStart: boolean): Promise<void> {
    this.storeAutoStart(autoStart);

    return IpcRendererEventChannel.autoStart.set(autoStart);
  }

  public setStartMinimized(startMinimized: boolean) {
    IpcRendererEventChannel.guiSettings.setStartMinimized(startMinimized);
  }

  public setMonochromaticIcon(monochromaticIcon: boolean) {
    IpcRendererEventChannel.guiSettings.setMonochromaticIcon(monochromaticIcon);
  }

  public setUnpinnedWindow(unpinnedWindow: boolean) {
    IpcRendererEventChannel.guiSettings.setUnpinnedWindow(unpinnedWindow);
  }

  public getLinuxSplitTunnelingApplications() {
    return IpcRendererEventChannel.linuxSplitTunneling.getApplications();
  }

  public getWindowsSplitTunnelingApplications(updateCache = false) {
    return IpcRendererEventChannel.windowsSplitTunneling.getApplications(updateCache);
  }

  public launchExcludedApplication(
    application: ILinuxSplitTunnelingApplication | string,
  ): Promise<LaunchApplicationResult> {
    return IpcRendererEventChannel.linuxSplitTunneling.launchApplication(application);
  }

  public setSplitTunnelingState = (enabled: boolean): Promise<void> => {
    return IpcRendererEventChannel.windowsSplitTunneling.setState(enabled);
  };

  public addSplitTunnelingApplication(application: IWindowsApplication | string): Promise<void> {
    return IpcRendererEventChannel.windowsSplitTunneling.addApplication(application);
  }

  public removeSplitTunnelingApplication(application: IWindowsApplication) {
    void IpcRendererEventChannel.windowsSplitTunneling.removeApplication(application);
  }

  public forgetManuallyAddedSplitTunnelingApplication(application: IWindowsApplication) {
    return IpcRendererEventChannel.windowsSplitTunneling.forgetManuallyAddedApplication(
      application,
    );
  }

  public collectProblemReport(toRedact?: string): Promise<string> {
    return IpcRendererEventChannel.problemReport.collectLogs(toRedact);
  }

  public async sendProblemReport(
    email: string,
    message: string,
    savedReportId: string,
  ): Promise<void> {
    await IpcRendererEventChannel.problemReport.sendReport({ email, message, savedReportId });
  }

  public viewLog(id: string): Promise<string> {
    return IpcRendererEventChannel.problemReport.viewLog(id);
  }

  public quit(): void {
    IpcRendererEventChannel.app.quit();
  }

  public openUrl(url: string): Promise<void> {
    return IpcRendererEventChannel.app.openUrl(url);
  }

  public showOpenDialog(
    options: Electron.OpenDialogOptions,
  ): Promise<Electron.OpenDialogReturnValue> {
    return IpcRendererEventChannel.app.showOpenDialog(options);
  }

  public getPreferredLocaleList(): IPreferredLocaleDescriptor[] {
    return [
      {
        // TRANSLATORS: The option that represents the active operating system language in the
        // TRANSLATORS: user interface language selection list.
        name: messages.gettext('System default'),
        code: SYSTEM_PREFERRED_LOCALE_KEY,
      },
      ...SUPPORTED_LOCALE_LIST.sort((a, b) => a.name.localeCompare(b.name)),
    ];
  }

  public async setPreferredLocale(preferredLocale: string): Promise<void> {
    const translations = await IpcRendererEventChannel.guiSettings.setPreferredLocale(
      preferredLocale,
    );

    // set current locale
    this.setLocale(translations.locale);

    // load translations for new locale
    loadTranslations(messages, translations.locale, translations.messages);
    loadTranslations(relayLocations, translations.locale, translations.relayLocations);
  }

  public getPreferredLocaleDisplayName(localeCode: string): string {
    const preferredLocale = this.getPreferredLocaleList().find((item) => item.code === localeCode);

    return preferredLocale ? preferredLocale.name : '';
  }

  public setDisplayedChangelog = (): void => {
    IpcRendererEventChannel.currentVersion.displayedChangelog();
  };

  public setNavigationHistory(history: IHistoryObject) {
    IpcRendererEventChannel.navigation.setHistory(history);
  }

  public setScrollPositions(scrollPositions: ScrollPositions) {
    IpcRendererEventChannel.navigation.setScrollPositions(scrollPositions);
  }

  // Make sure that the content height is correct and log if it isn't. This is mostly for debugging
  // purposes since there's a bug in Electron that causes the app height to be another value than
  // the one we have set.
  // https://github.com/electron/electron/issues/28777
  private checkContentHeight(resize: boolean): void {
    let expectedContentHeight = 568;

    // The app content is 12px taller on macOS to fit the top arrow.
    if (window.env.platform === 'darwin' && !this.guiSettings.unpinnedWindow) {
      expectedContentHeight += 12;
    }

    const contentHeight = window.innerHeight;
    if (contentHeight !== expectedContentHeight) {
      log.verbose(
        resize ? 'Resize:' : 'Initial:',
        `Wrong content height: ${contentHeight}, expected ${expectedContentHeight}`,
      );
    }
  }

  private redirectToConnect() {
    // Redirect the user after some time to allow for the 'Logged in' screen to be visible
    this.loginScheduler.schedule(() => this.resetNavigation(), 1000);
  }

  private setLocale(locale: string) {
    this.reduxActions.userInterface.updateLocale(locale);
  }

  private setRelaySettings(relaySettings: RelaySettings) {
    const actions = this.reduxActions;

    if ('normal' in relaySettings) {
      const {
        location,
        openvpnConstraints,
        wireguardConstraints,
        tunnelProtocol,
        providers,
      } = relaySettings.normal;

      actions.settings.updateRelay({
        normal: {
          location: liftConstraint(location),
          providers,
          openvpn: {
            port: liftConstraint(openvpnConstraints.port),
            protocol: liftConstraint(openvpnConstraints.protocol),
          },
          wireguard: {
            port: liftConstraint(wireguardConstraints.port),
            ipVersion: liftConstraint(wireguardConstraints.ipVersion),
            useMultihop: wireguardConstraints.useMultihop,
            entryLocation: liftConstraint(wireguardConstraints.entryLocation),
          },
          tunnelProtocol: liftConstraint(tunnelProtocol),
        },
      });
    } else if ('customTunnelEndpoint' in relaySettings) {
      const customTunnelEndpoint = relaySettings.customTunnelEndpoint;
      const config = customTunnelEndpoint.config;

      if ('openvpn' in config) {
        actions.settings.updateRelay({
          customTunnelEndpoint: {
            host: customTunnelEndpoint.host,
            port: config.openvpn.endpoint.port,
            protocol: config.openvpn.endpoint.protocol,
          },
        });
      } else if ('wireguard' in config) {
        // TODO: handle wireguard
      }
    }
  }

  private setBridgeSettings(bridgeSettings: BridgeSettings) {
    const actions = this.reduxActions;

    if ('normal' in bridgeSettings) {
      actions.settings.updateBridgeSettings({
        normal: {
          location: liftConstraint(bridgeSettings.normal.location),
        },
      });
    } else if ('custom' in bridgeSettings) {
      actions.settings.updateBridgeSettings({
        custom: bridgeSettings.custom,
      });
    }
  }

  private onDaemonConnected() {
    this.connectedToDaemon = true;
    this.reduxActions.userInterface.setConnectedToDaemon(true);
    this.resetNavigation();
  }

  private onDaemonDisconnected() {
    this.connectedToDaemon = false;
    this.reduxActions.userInterface.setConnectedToDaemon(false);
    this.resetNavigation();
  }

  private resetNavigation() {
    if (this.history) {
      const pathname = this.history.location.pathname as RoutePath;
      const nextPath = this.getNavigationBase() as RoutePath;

      if (pathname !== nextPath) {
        // First level contains the possible next locations and the second level contains the
        // possible current locations.
        const navigationTransitions: Partial<
          Record<RoutePath, Partial<Record<RoutePath | '*', ITransitionSpecification>>>
        > = {
          [RoutePath.launch]: {
            [RoutePath.login]: transitions.pop,
            [RoutePath.main]: transitions.pop,
            '*': transitions.dismiss,
          },
          [RoutePath.login]: {
            [RoutePath.launch]: transitions.push,
            [RoutePath.main]: transitions.pop,
            [RoutePath.deviceRevoked]: transitions.pop,
            '*': transitions.none,
          },
          [RoutePath.main]: {
            [RoutePath.launch]: transitions.push,
            [RoutePath.login]: transitions.push,
            [RoutePath.tooManyDevices]: transitions.push,
            '*': transitions.dismiss,
          },
          [RoutePath.deviceRevoked]: {
            '*': transitions.pop,
          },
        };

        const transition =
          navigationTransitions[nextPath]?.[pathname] ?? navigationTransitions[nextPath]?.['*'];
        this.history.reset(nextPath, transition);
      }
    }
  }

  private getNavigationBase(): RoutePath {
    if (this.connectedToDaemon && this.hasReceivedDeviceConfig) {
      const loginState = this.reduxStore.getState().account.status;
      const deviceRevoked = loginState.type === 'none' && loginState.deviceRevoked;

      if (deviceRevoked) {
        return RoutePath.deviceRevoked;
      } else if (this.deviceConfig?.accountToken) {
        return RoutePath.main;
      } else {
        return RoutePath.login;
      }
    } else {
      return RoutePath.launch;
    }
  }

  private setAccountHistory(accountHistory?: AccountToken) {
    this.reduxActions.account.updateAccountHistory(accountHistory);
  }

  private setTunnelState(tunnelState: TunnelState) {
    const actions = this.reduxActions;

    log.verbose(`Tunnel state: ${tunnelState.state}`);

    this.tunnelState = tunnelState;

    batch(() => {
      switch (tunnelState.state) {
        case 'connecting':
          actions.connection.connecting(tunnelState.details);
          break;

        case 'connected':
          actions.connection.connected(tunnelState.details);
          break;

        case 'disconnecting':
          actions.connection.disconnecting(tunnelState.details);
          break;

        case 'disconnected':
          actions.connection.disconnected();
          break;

        case 'error':
          actions.connection.blocked(tunnelState.details);
          break;
      }

      // Update the location when entering a new tunnel state since it's likely changed.
      void this.updateLocation();
    });
  }

  private setSettings(newSettings: ISettings) {
    this.settings = newSettings;

    const reduxSettings = this.reduxActions.settings;

    reduxSettings.updateAllowLan(newSettings.allowLan);
    reduxSettings.updateEnableIpv6(newSettings.tunnelOptions.generic.enableIpv6);
    reduxSettings.updateBlockWhenDisconnected(newSettings.blockWhenDisconnected);
    reduxSettings.updateShowBetaReleases(newSettings.showBetaReleases);
    reduxSettings.updateOpenVpnMssfix(newSettings.tunnelOptions.openvpn.mssfix);
    reduxSettings.updateWireguardMtu(newSettings.tunnelOptions.wireguard.mtu);
    reduxSettings.updateBridgeState(newSettings.bridgeState);
    reduxSettings.updateDnsOptions(newSettings.tunnelOptions.dns);
    reduxSettings.updateSplitTunnelingState(newSettings.splitTunnel.enableExclusions);

    this.setRelaySettings(newSettings.relaySettings);
    this.setBridgeSettings(newSettings.bridgeSettings);
  }

  private setIsPerformingPostUpgrade(isPerformingPostUpgrade: boolean) {
    this.reduxActions.userInterface.setIsPerformingPostUpgrade(isPerformingPostUpgrade);
  }

  private updateBlockedState(tunnelState: TunnelState, blockWhenDisconnected: boolean) {
    const actions = this.reduxActions.connection;
    switch (tunnelState.state) {
      case 'connecting':
        actions.updateBlockState(true);
        break;

      case 'connected':
        actions.updateBlockState(false);
        break;

      case 'disconnected':
        actions.updateBlockState(blockWhenDisconnected);
        break;

      case 'disconnecting':
        actions.updateBlockState(true);
        break;

      case 'error':
        actions.updateBlockState(!tunnelState.details.blockFailure);
        break;
    }
  }

  private handleAccountChange(
    newDeviceEvent: IDeviceEvent,
    oldAccount?: string,
    preventRedirectToConnect?: boolean,
  ) {
    const reduxAccount = this.reduxActions.account;

    this.deviceConfig = newDeviceEvent.deviceConfig;
    const newAccount = newDeviceEvent.deviceConfig?.accountToken;
    const newDevice = newDeviceEvent.deviceConfig?.device;

    if (oldAccount && !newAccount) {
      this.loginScheduler.cancel();
      if (!this.reduxStore.getState().account.loggingOut && newDeviceEvent.remote) {
        reduxAccount.deviceRevoked();
      } else {
        reduxAccount.loggedOut();
      }

      this.resetNavigation();
    } else if (newAccount !== undefined && newDevice !== undefined && oldAccount !== newAccount) {
      switch (this.loginState) {
        case 'none':
        case 'logging in':
          reduxAccount.loggedIn({ accountToken: newAccount, device: newDevice });

          if (this.previousLoginState === 'too many devices') {
            this.resetNavigation();
          } else if (!preventRedirectToConnect) {
            this.redirectToConnect();
          }
          break;
        case 'creating account':
          reduxAccount.accountCreated(
            { accountToken: newAccount, device: newDevice },
            new Date().toISOString(),
          );
          break;
      }

      if (this.loginState !== 'logging in' && this.loginState !== 'creating account') {
        this.resetNavigation();
      }
    }

    this.previousLoginState = this.loginState;
    this.loginState = 'none';
  }

  private setLocation(location: Partial<ILocation>) {
    this.location = location;
    this.propagateLocationToRedux();
  }

  private propagateLocationToRedux() {
    if (this.location) {
      this.reduxActions.connection.newLocation(this.location);
    }
  }

  private setRelayListPair(relayListPair: IRelayListPair) {
    this.relayListPair = relayListPair;
    this.propagateRelayListPairToRedux();
  }

  private propagateRelayListPairToRedux() {
    const relays = this.relayListPair.relays.countries;
    const bridges = this.relayListPair.bridges.countries;

    this.reduxActions.settings.updateRelayLocations(relays);
    this.reduxActions.settings.updateBridgeLocations(bridges);
  }

  private setCurrentVersion(versionInfo: ICurrentAppVersionInfo) {
    this.reduxActions.version.updateVersion(
      versionInfo.gui,
      versionInfo.isConsistent,
      versionInfo.isBeta,
    );
  }

  private setUpgradeVersion(upgradeVersion: IAppVersionInfo) {
    this.reduxActions.version.updateLatest(upgradeVersion);
  }

  private setGuiSettings(guiSettings: IGuiSettingsState) {
    this.guiSettings = guiSettings;
    this.reduxActions.settings.updateGuiSettings(guiSettings);
  }

  private setAccountExpiry(expiry?: string) {
    this.reduxActions.account.updateAccountExpiry(expiry);
  }

  private storeAutoStart(autoStart: boolean) {
    this.reduxActions.settings.updateAutoStart(autoStart);
  }

  private setChangelog(changelog: IChangelog) {
    this.reduxActions.userInterface.setChangelog(changelog);
  }

  private async updateLocation() {
    switch (this.tunnelState.state) {
      case 'disconnected': {
        if (this.lastDisconnectedLocation) {
          this.setLocation(this.lastDisconnectedLocation);
        }
        const location = await this.fetchLocation();
        if (location) {
          this.setLocation(location);
          this.lastDisconnectedLocation = location;
        }
        break;
      }
      case 'disconnecting':
        if (this.lastDisconnectedLocation) {
          this.setLocation(this.lastDisconnectedLocation);
        } else {
          // If there's no previous location while disconnecting we remove the location. We keep the
          // coordinates to prevent the map from jumping around.
          const { longitude, latitude } = this.reduxStore.getState().connection;
          this.setLocation({ longitude, latitude });
        }
        break;
      case 'connecting':
        this.setLocation(this.tunnelState.details?.location ?? this.getLocationFromConstraints());
        break;
      case 'connected': {
        if (this.tunnelState.details?.location) {
          this.setLocation(this.tunnelState.details.location);
        }
        const location = await this.fetchLocation();
        if (location) {
          this.setLocation(location);
        }
        break;
      }
    }
  }

  private async fetchLocation(): Promise<ILocation | void> {
    try {
      // Fetch the new user location
      const getLocationPromise = IpcRendererEventChannel.location.get();
      this.getLocationPromise = getLocationPromise;
      const location = await getLocationPromise;
      // If the location is currently unavailable, do nothing! This only ever happens when a
      // custom relay is set or we are in a blocked state.
      if (location && getLocationPromise === this.getLocationPromise) {
        return location;
      }
    } catch (e) {
      const error = e as Error;
      log.error(`Failed to update the location: ${error.message}`);
    }
  }

  private getLocationFromConstraints(): Partial<ILocation> {
    const state = this.reduxStore.getState();
    const coordinates = {
      longitude: state.connection.longitude,
      latitude: state.connection.latitude,
    };

    const relaySettings = this.settings.relaySettings;
    if ('normal' in relaySettings) {
      const location = relaySettings.normal.location;
      if (location !== 'any' && 'only' in location) {
        const constraint = location.only;

        const relayLocations = state.settings.relayLocations;
        if ('country' in constraint) {
          const country = relayLocations.find(({ code }) => constraint.country === code);

          return { country: country?.name, ...coordinates };
        } else if ('city' in constraint) {
          const country = relayLocations.find(({ code }) => constraint.city[0] === code);
          const city = country?.cities.find(({ code }) => constraint.city[1] === code);

          return { country: country?.name, city: city?.name, ...coordinates };
        } else if ('hostname' in constraint) {
          const country = relayLocations.find(({ code }) => constraint.hostname[0] === code);
          const city = country?.cities.find((location) => location.code === constraint.hostname[1]);

          let entryHostname: string | undefined;
          const multihopConstraint = relaySettings.normal.wireguardConstraints.useMultihop;
          const entryLocationConstraint = relaySettings.normal.wireguardConstraints.entryLocation;
          if (
            multihopConstraint &&
            entryLocationConstraint !== 'any' &&
            'hostname' in entryLocationConstraint.only &&
            entryLocationConstraint.only.hostname.length === 3
          ) {
            entryHostname = entryLocationConstraint.only.hostname[2];
          }

          return {
            country: country?.name,
            city: city?.name,
            hostname: constraint.hostname[2],
            entryHostname,
            ...coordinates,
          };
        }
      }
    }

    return coordinates;
  }
}
