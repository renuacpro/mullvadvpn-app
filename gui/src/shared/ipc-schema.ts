import { GetTextTranslations } from 'gettext-parser';

import { ILinuxSplitTunnelingApplication, IWindowsApplication } from './application-types';
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
  IRelayList,
  ISettings,
  RelaySettingsUpdate,
  TunnelState,
  VoucherResponse,
} from './daemon-rpc-types';
import { IGuiSettingsState } from './gui-settings-state';
import { LogLevel } from './logging-types';

interface ILogEntry {
  level: LogLevel;
  message: string;
}
import { invoke, invokeSync, notifyRenderer, send } from './ipc-helpers';
import {
  IChangelog,
  ICurrentAppVersionInfo,
  IHistoryObject,
  IWindowShapeParameters,
  ScrollPositions,
} from './ipc-types';

export interface ITranslations {
  locale: string;
  messages?: GetTextTranslations;
  relayLocations?: GetTextTranslations;
}

export interface IRelayListPair {
  relays: IRelayList;
  bridges: IRelayList;
}

export type LaunchApplicationResult = { success: true } | { error: string };

export enum MacOsScrollbarVisibility {
  always,
  whenScrolling,
  automatic,
}

export interface IAppStateSnapshot {
  isConnected: boolean;
  autoStart: boolean;
  accountData?: IAccountData;
  accountHistory?: AccountToken;
  tunnelState: TunnelState;
  settings: ISettings;
  isPerformingPostUpgrade: boolean;
  deviceConfig?: IDeviceConfig;
  hasReceivedDeviceConfig: boolean;
  relayListPair: IRelayListPair;
  currentVersion: ICurrentAppVersionInfo;
  upgradeVersion: IAppVersionInfo;
  guiSettings: IGuiSettingsState;
  translations: ITranslations;
  windowsSplitTunnelingApplications?: IWindowsApplication[];
  macOsScrollbarVisibility?: MacOsScrollbarVisibility;
  changelog: IChangelog;
  navigationHistory?: IHistoryObject;
  scrollPositions: ScrollPositions;
}

// The different types of requests are:
// * send<ArgumentType>(), which is used for one-way communication from the renderer process to the
//    main process. The main channel will have a property named 'handle<PropertyName>' and the
//    renderer will have a property named the same as the one specified.
// * invoke<ArgumentType, ReturnType>(), which is used for two-way communication from the renderer
//    process to the main process. The naming is the same as `send<A>()`.
// * invokeSync<ArgumentType, ReturnType>(), same as `invoke<A, R>()` but synchronous.
// * notifyRenderer<ArgumentType>(), which is used for one-way communication from the main process
//    to the renderer process. The renderer ipc channel will have a property named
//    `listen<PropertyName>` and the main channel will have a property named `notify<PropertyName>`.
//
// Example:
// const ipc = {
//   groupOfCalls: {
//     first: send<boolean>(),
//     second: request<boolean, number>(),
//     third: requestSync<boolean, number>(),
//     fourth: notifyRenderer<boolean>(),
//   },
// };
//
// createIpcMain(ipc)
//   => {
//     groupOfCalls: {
//       handleFirst: (fn: (arg: boolean) => void) => void,
//       handleSecond: (fn: (arg: boolean) => Promise<number>) => void,
//       handleThird: (fn: (arg: boolean) => number) => void,
//       notifyFourth: (arg: boolean) => void,
//     },
//
// createIpcRenderer(ipc)
//   => {
//     groupOfCalls: {
//       first: (arg: boolean) => void,
//       second: (arg: boolean) => Promise<number>,
//       third: (arg: boolean) => number,
//       listenFourth: (fn: (arg: boolean) => void) => void,
//     },
//   }
export const ipcSchema = {
  state: {
    get: invokeSync<void, IAppStateSnapshot>(),
  },
  window: {
    shape: notifyRenderer<IWindowShapeParameters>(),
    focus: notifyRenderer<boolean>(),
    macOsScrollbarVisibility: notifyRenderer<MacOsScrollbarVisibility>(),
  },
  navigation: {
    reset: notifyRenderer<void>(),
    setHistory: send<IHistoryObject>(),
    setScrollPositions: send<ScrollPositions>(),
  },
  daemon: {
    isPerformingPostUpgrade: notifyRenderer<boolean>(),
    connected: notifyRenderer<void>(),
    disconnected: notifyRenderer<void>(),
  },
  relays: {
    '': notifyRenderer<IRelayListPair>(),
  },
  currentVersion: {
    '': notifyRenderer<ICurrentAppVersionInfo>(),
    displayedChangelog: send<void>(),
  },
  upgradeVersion: {
    '': notifyRenderer<IAppVersionInfo>(),
  },
  app: {
    quit: send<void>(),
    openUrl: invoke<string, void>(),
    showOpenDialog: invoke<Electron.OpenDialogOptions, Electron.OpenDialogReturnValue>(),
  },
  location: {
    get: invoke<void, ILocation>(),
  },
  tunnel: {
    '': notifyRenderer<TunnelState>(),
    connect: invoke<void, void>(),
    disconnect: invoke<void, void>(),
    reconnect: invoke<void, void>(),
  },
  settings: {
    '': notifyRenderer<ISettings>(),
    setAllowLan: invoke<boolean, void>(),
    setShowBetaReleases: invoke<boolean, void>(),
    setEnableIpv6: invoke<boolean, void>(),
    setBlockWhenDisconnected: invoke<boolean, void>(),
    setBridgeState: invoke<BridgeState, void>(),
    setOpenVpnMssfix: invoke<number | undefined, void>(),
    setWireguardMtu: invoke<number | undefined, void>(),
    updateRelaySettings: invoke<RelaySettingsUpdate, void>(),
    updateBridgeSettings: invoke<BridgeSettings, void>(),
    setDnsOptions: invoke<IDnsOptions, void>(),
  },
  guiSettings: {
    '': notifyRenderer<IGuiSettingsState>(),
    setEnableSystemNotifications: send<boolean>(),
    setAutoConnect: send<boolean>(),
    setStartMinimized: send<boolean>(),
    setMonochromaticIcon: send<boolean>(),
    setPreferredLocale: invoke<string, ITranslations>(),
    setUnpinnedWindow: send<boolean>(),
  },
  account: {
    '': notifyRenderer<IAccountData | undefined>(),
    device: notifyRenderer<IDeviceEvent>(),
    devices: notifyRenderer<Array<IDevice>>(),
    create: invoke<void, string>(),
    login: invoke<AccountToken, void>(),
    logout: invoke<void, void>(),
    getWwwAuthToken: invoke<void, string>(),
    submitVoucher: invoke<string, VoucherResponse>(),
    updateData: send<void>(),
    getDevice: invoke<void, IDevice | undefined>(),
    listDevices: invoke<AccountToken, Array<IDevice>>(),
    removeDevice: invoke<IDeviceRemoval, void>(),
  },
  accountHistory: {
    '': notifyRenderer<AccountToken | undefined>(),
    clear: invoke<void, void>(),
  },
  autoStart: {
    '': notifyRenderer<boolean>(),
    set: invoke<boolean, void>(),
  },
  problemReport: {
    collectLogs: invoke<string | undefined, string>(),
    sendReport: invoke<{ email: string; message: string; savedReportId: string }, void>(),
    viewLog: invoke<string, string>(),
  },
  logging: {
    log: send<ILogEntry>(),
  },
  linuxSplitTunneling: {
    getApplications: invoke<void, ILinuxSplitTunnelingApplication[]>(),
    launchApplication: invoke<ILinuxSplitTunnelingApplication | string, LaunchApplicationResult>(),
  },
  windowsSplitTunneling: {
    '': notifyRenderer<IWindowsApplication[]>(),
    setState: invoke<boolean, void>(),
    getApplications: invoke<boolean, { fromCache: boolean; applications: IWindowsApplication[] }>(),
    addApplication: invoke<IWindowsApplication | string, void>(),
    removeApplication: invoke<IWindowsApplication, void>(),
    forgetManuallyAddedApplication: invoke<IWindowsApplication, void>(),
  },
};
