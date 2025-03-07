import { MacOsScrollbarVisibility } from '../../../shared/ipc-schema';
import { IChangelog, ScrollPositions } from '../../../shared/ipc-types';

export interface IUpdateLocaleAction {
  type: 'UPDATE_LOCALE';
  locale: string;
}

export interface IUpdateWindowArrowPositionAction {
  type: 'UPDATE_WINDOW_ARROW_POSITION';
  arrowPosition: number;
}

export interface IUpdateConnectionInfoOpenAction {
  type: 'TOGGLE_CONNECTION_PANEL';
}

export interface ISetWindowFocusedAction {
  type: 'SET_WINDOW_FOCUSED';
  focused: boolean;
}

export interface ISetScrollPositions {
  type: 'SET_SCROLL_POSITIONS';
  scrollPositions: ScrollPositions;
}

export interface IAddScrollPosition {
  type: 'ADD_SCROLL_POSITION';
  path: string;
  scrollPosition: [number, number];
}

export interface IRemoveScrollPosition {
  type: 'REMOVE_SCROLL_POSITION';
  path: string;
}

export interface ISetMacOsScrollbarVisibility {
  type: 'SET_MACOS_SCROLLBAR_VISIBILITY';
  visibility: MacOsScrollbarVisibility;
}

export interface ISetConnectedToDaemon {
  type: 'SET_CONNECTED_TO_DAEMON';
  connectedToDaemon: boolean;
}

export interface ISetChangelog {
  type: 'SET_CHANGELOG';
  changelog: IChangelog;
}

export interface ISetIsPerformingPostUpgrade {
  type: 'SET_IS_PERFORMING_POST_UPGRADE';
  isPerformingPostUpgrade: boolean;
}

export type UserInterfaceAction =
  | IUpdateLocaleAction
  | IUpdateWindowArrowPositionAction
  | IUpdateConnectionInfoOpenAction
  | ISetWindowFocusedAction
  | ISetScrollPositions
  | IAddScrollPosition
  | IRemoveScrollPosition
  | ISetMacOsScrollbarVisibility
  | ISetConnectedToDaemon
  | ISetChangelog
  | ISetIsPerformingPostUpgrade;

function updateLocale(locale: string): IUpdateLocaleAction {
  return {
    type: 'UPDATE_LOCALE',
    locale,
  };
}

function updateWindowArrowPosition(arrowPosition: number): IUpdateWindowArrowPositionAction {
  return {
    type: 'UPDATE_WINDOW_ARROW_POSITION',
    arrowPosition,
  };
}

function toggleConnectionPanel(): IUpdateConnectionInfoOpenAction {
  return {
    type: 'TOGGLE_CONNECTION_PANEL',
  };
}

function setWindowFocused(focused: boolean): ISetWindowFocusedAction {
  return {
    type: 'SET_WINDOW_FOCUSED',
    focused,
  };
}

function setScrollPositions(scrollPositions: ScrollPositions): ISetScrollPositions {
  return {
    type: 'SET_SCROLL_POSITIONS',
    scrollPositions,
  };
}

function addScrollPosition(path: string, scrollPosition: [number, number]): IAddScrollPosition {
  return {
    type: 'ADD_SCROLL_POSITION',
    path,
    scrollPosition,
  };
}

function removeScrollPosition(path: string): IRemoveScrollPosition {
  return {
    type: 'REMOVE_SCROLL_POSITION',
    path,
  };
}

function setMacOsScrollbarVisibility(
  visibility: MacOsScrollbarVisibility,
): ISetMacOsScrollbarVisibility {
  return {
    type: 'SET_MACOS_SCROLLBAR_VISIBILITY',
    visibility,
  };
}

function setConnectedToDaemon(connectedToDaemon: boolean): ISetConnectedToDaemon {
  return {
    type: 'SET_CONNECTED_TO_DAEMON',
    connectedToDaemon,
  };
}

function setChangelog(changelog: IChangelog): ISetChangelog {
  return {
    type: 'SET_CHANGELOG',
    changelog,
  };
}

function setIsPerformingPostUpgrade(isPerformingPostUpgrade: boolean): ISetIsPerformingPostUpgrade {
  return {
    type: 'SET_IS_PERFORMING_POST_UPGRADE',
    isPerformingPostUpgrade,
  };
}

export default {
  updateLocale,
  updateWindowArrowPosition,
  toggleConnectionPanel,
  setWindowFocused,
  setScrollPositions,
  addScrollPosition,
  removeScrollPosition,
  setMacOsScrollbarVisibility,
  setConnectedToDaemon,
  setChangelog,
  setIsPerformingPostUpgrade,
};
