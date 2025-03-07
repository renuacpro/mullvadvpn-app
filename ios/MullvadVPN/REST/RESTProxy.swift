//
//  RESTProxy.swift
//  MullvadVPN
//
//  Created by pronebird on 20/04/2022.
//  Copyright © 2022 Mullvad VPN AB. All rights reserved.
//

import Foundation

extension REST {
    class Proxy<ConfigurationType: ProxyConfiguration> {
        typealias CompletionHandler<Success> = (OperationCompletion<Success, REST.Error>) -> Void

        /// Synchronization queue used by network operations.
        let dispatchQueue: DispatchQueue

        /// Operation queue used for running network operations.
        let operationQueue = OperationQueue()

        /// Proxy configuration.
        let configuration: ConfigurationType

        /// URL request factory.
        let requestFactory: REST.RequestFactory

        /// URL response decoder.
        let responseDecoder: REST.ResponseDecoder

        init(
            name: String,
            configuration: ConfigurationType,
            requestFactory: REST.RequestFactory,
            responseDecoder: REST.ResponseDecoder
        )
        {
            dispatchQueue = DispatchQueue(label: "REST.\(name).dispatchQueue")
            operationQueue.name = "REST.\(name).operationQueue"

            self.configuration = configuration
            self.requestFactory = requestFactory
            self.responseDecoder = responseDecoder
        }

        func addOperation<Success>(
            name: String,
            retryStrategy: REST.RetryStrategy,
            requestHandler: REST.AnyRequestHandler,
            responseHandler: REST.AnyResponseHandler<Success>,
            completionHandler: @escaping NetworkOperation<Success>.CompletionHandler
        ) -> Cancellable
        {
            let operation = NetworkOperation(
                name: getTaskIdentifier(name: name),
                dispatchQueue: dispatchQueue,
                configuration: configuration,
                retryStrategy: retryStrategy,
                requestHandler: requestHandler,
                responseHandler: responseHandler,
                completionHandler: completionHandler
            )

            operationQueue.addOperation(operation)

            return operation
        }
    }

    class ProxyConfiguration {
        let session: URLSession
        let addressCacheStore: AddressCache.Store

        init(session: URLSession, addressCacheStore: AddressCache.Store) {
            self.session = session
            self.addressCacheStore = addressCacheStore
        }
    }

    class AuthProxyConfiguration: ProxyConfiguration {
        let accessTokenManager: AccessTokenManager

        init(proxyConfiguration: ProxyConfiguration, accessTokenManager: AccessTokenManager) {
            self.accessTokenManager = accessTokenManager

            super.init(
                session: proxyConfiguration.session,
                addressCacheStore: proxyConfiguration.addressCacheStore
            )
        }
    }
}
