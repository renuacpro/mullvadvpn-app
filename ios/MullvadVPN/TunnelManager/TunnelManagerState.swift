//
//  TunnelManager.State.swift
//  MullvadVPN
//
//  Created by pronebird on 26/01/2022.
//  Copyright © 2022 Mullvad VPN AB. All rights reserved.
//

import Foundation
import NetworkExtension

protocol TunnelManagerStateDelegate: AnyObject {
    func tunnelManagerState(_ state: TunnelManager.State, didChangeTunnelInfo newTunnelInfo: TunnelInfo?)
    func tunnelManagerState(_ state: TunnelManager.State, didChangeTunnelStatus newTunnelStatus: TunnelStatus)
    func tunnelManagerState(_ state: TunnelManager.State, didChangeTunnelProvider newTunnelObject: Tunnel?, shouldRefreshTunnelState: Bool)
}

extension TunnelManager {

    class State {
        let queue: DispatchQueue
        weak var delegate: TunnelManagerStateDelegate?

        private let queueMarkerKey = DispatchSpecificKey<Bool>()

        private var _tunnelInfo: TunnelInfo?
        private var _tunnelObject: Tunnel?
        private var _tunnelStatus = TunnelStatus(
            isNetworkReachable: false,
            connectingDate: nil,
            state: .disconnected
        )

        var tunnelInfo: TunnelInfo? {
            get {
                return performBlock {
                    return _tunnelInfo
                }
            }
            set {
                performBlock {
                    if _tunnelInfo != newValue {
                        _tunnelInfo = newValue

                        delegate?.tunnelManagerState(self, didChangeTunnelInfo: newValue)
                    }
                }
            }
        }

        var tunnel: Tunnel? {
            return performBlock {
                return _tunnelObject
            }
        }

        var tunnelStatus: TunnelStatus {
            get {
                return performBlock {
                    return _tunnelStatus
                }
            }
            set {
                performBlock {
                    if _tunnelStatus != newValue {
                        _tunnelStatus = newValue

                        delegate?.tunnelManagerState(self, didChangeTunnelStatus: newValue)
                    }
                }
            }
        }

        init(queue: DispatchQueue) {
            self.queue = queue

            queue.setSpecific(key: queueMarkerKey, value: true)
        }

        deinit {
            queue.setSpecific(key: queueMarkerKey, value: nil)
        }

        func setTunnel(_ newTunnelObject: Tunnel?, shouldRefreshTunnelState: Bool) {
            performBlock {
                if _tunnelObject != newTunnelObject {
                    _tunnelObject = newTunnelObject

                    delegate?.tunnelManagerState(self, didChangeTunnelProvider: newTunnelObject, shouldRefreshTunnelState: shouldRefreshTunnelState)
                }
            }
        }

        private func performBlock<T>(_ block: () -> T) -> T {
            let isTargetQueue = DispatchQueue.getSpecific(key: queueMarkerKey) ?? false

            if isTargetQueue {
                return block()
            } else {
                return queue.sync(execute: block)
            }
        }
    }
}
