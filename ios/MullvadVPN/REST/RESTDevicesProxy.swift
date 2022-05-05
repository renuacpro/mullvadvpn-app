//
//  RESTDevicesProxy.swift
//  MullvadVPN
//
//  Created by pronebird on 20/04/2022.
//  Copyright © 2022 Mullvad VPN AB. All rights reserved.
//

import Foundation
import class WireGuardKitTypes.PublicKey
import struct WireGuardKitTypes.IPAddressRange

extension REST {
    class DevicesProxy: Proxy<AuthProxyConfiguration> {
        init(configuration: AuthProxyConfiguration) {
            super.init(
                name: "DevicesProxy",
                configuration: configuration,
                requestFactory: RequestFactory.withDefaultAPICredentials(
                    pathPrefix: "/accounts/v1-beta1",
                    bodyEncoder: Coding.makeJSONEncoder()
                ),
                responseDecoder: Coding.makeJSONDecoder()
            )
        }

        /// Fetch device by identifier.
        /// The completion handler receives `nil` if device is not found.
        func getDevice(
            accountNumber: String,
            identifier: String,
            retryStrategy: REST.RetryStrategy,
            completion: @escaping CompletionHandler<Device?>
        ) -> Cancellable
        {
            let requestHandler = AnyRequestHandler(
                createURLRequest: { endpoint, authorization in
                    let urlEncodedIdentifier = identifier
                        .addingPercentEncoding(withAllowedCharacters: .alphanumerics)!
                    let path = "device/\(urlEncodedIdentifier)"

                    var requestBuilder = self.requestFactory.createURLRequestBuilder(
                        endpoint: endpoint,
                        method: .get,
                        path: path
                    )

                    requestBuilder.setAuthorization(authorization)

                    return requestBuilder.getURLRequest()
                },
                requestAuthorization: { completion in
                    return self.configuration.accessTokenManager
                        .getAccessToken(
                            accountNumber: accountNumber,
                            retryStrategy: retryStrategy
                        ) { operationCompletion in
                            completion(operationCompletion.map { tokenData in
                                return .accessToken(tokenData.accessToken)
                            })
                        }
                }
            )

            let responseHandler = AnyResponseHandler { response, data -> ResponseHandlerResult<Device?> in
                let httpStatus = HTTPStatus(rawValue: response.statusCode)

                switch httpStatus {
                case let httpStatus where httpStatus.isSuccess:
                    return .decoding {
                        return try self.responseDecoder.decode(Device.self, from: data)
                    }

                case .notFound:
                    return .success(nil)

                default:
                    return .unhandledResponse(
                        try? self.responseDecoder.decode(
                            ServerErrorResponse.self,
                            from: data
                        )
                    )
                }
            }

            return addOperation(
                name: "get-device",
                retryStrategy: retryStrategy,
                requestHandler: requestHandler,
                responseHandler: responseHandler,
                completionHandler: completion
            )
        }

        /// Fetch a list of created devices.
        func getDevices(
            accountNumber: String,
            retryStrategy: REST.RetryStrategy,
            completion: @escaping CompletionHandler<[Device]>
        ) -> Cancellable
        {
            let requestHandler = AnyRequestHandler(
                createURLRequest: { endpoint, authorization in
                    var requestBuilder = self.requestFactory.createURLRequestBuilder(
                        endpoint: endpoint,
                        method: .get,
                        path: "devices"
                    )

                    requestBuilder.setAuthorization(authorization)

                    return requestBuilder.getURLRequest()
                },
                requestAuthorization: { completion in
                    return self.configuration.accessTokenManager
                        .getAccessToken(
                            accountNumber: accountNumber,
                            retryStrategy: retryStrategy
                        ) { operationCompletion in
                            completion(operationCompletion.map { tokenData in
                                return .accessToken(tokenData.accessToken)
                            })
                        }
                }
            )

            let responseHandler = REST.defaultResponseHandler(
                decoding: [Device].self,
                with: responseDecoder
            )

            return addOperation(
                name: "get-devices",
                retryStrategy: retryStrategy,
                requestHandler: requestHandler,
                responseHandler: responseHandler,
                completionHandler: completion
            )
        }

        /// Create new device.
        /// The completion handler will receive a `CreateDeviceResponse.created(Device)` on success.
        /// Other `CreateDeviceResponse` variants describe errors.
        func createDevice(
            accountNumber: String,
            request: CreateDeviceRequest,
            retryStrategy: REST.RetryStrategy,
            completion: @escaping CompletionHandler<Device>
        ) -> Cancellable
        {
            let requestHandler = AnyRequestHandler(
                createURLRequest: { endpoint, authorization in
                    var requestBuilder = self.requestFactory.createURLRequestBuilder(
                        endpoint: endpoint,
                        method: .post,
                        path: "devices"
                    )
                    requestBuilder.setAuthorization(authorization)

                    try requestBuilder.setHTTPBody(value: request)

                    return requestBuilder.getURLRequest()
                },
                requestAuthorization: { completion in
                    return self.configuration.accessTokenManager
                        .getAccessToken(
                            accountNumber: accountNumber,
                            retryStrategy: retryStrategy
                        ) { operationCompletion in
                            completion(operationCompletion.map { tokenData in
                                return .accessToken(tokenData.accessToken)
                            })
                        }
                }
            )

            let responseHandler = REST.defaultResponseHandler(
                decoding: Device.self,
                with: responseDecoder
            )

            return addOperation(
                name: "create-device",
                retryStrategy: retryStrategy,
                requestHandler: requestHandler,
                responseHandler: responseHandler,
                completionHandler: completion
            )
        }

        /// Delete device by identifier.
        /// The completion handler will receive `true` if device is successfully removed,
        /// otherwise `false` if device is not found or already removed.
        func deleteDevice(
            accountNumber: String,
            identifier: String,
            retryStrategy: REST.RetryStrategy,
            completion: @escaping CompletionHandler<Bool>
        ) -> Cancellable
        {
            let requestHandler = AnyRequestHandler(
                createURLRequest: { endpoint, authorization in
                    let urlEncodedIdentifier = identifier
                        .addingPercentEncoding(withAllowedCharacters: .alphanumerics)!
                    let path = "devices/".appending(urlEncodedIdentifier)

                    var requestBuilder = self.requestFactory
                        .createURLRequestBuilder(
                            endpoint: endpoint,
                            method: .delete,
                            path: path
                        )

                    requestBuilder.setAuthorization(authorization)

                    return requestBuilder.getURLRequest()
                },
                requestAuthorization: { completion in
                    return self.configuration.accessTokenManager
                        .getAccessToken(
                            accountNumber: accountNumber,
                            retryStrategy: retryStrategy
                        ) { operationCompletion in
                            completion(operationCompletion.map { tokenData in
                                return .accessToken(tokenData.accessToken)
                            })
                        }
                }
            )

            let responseHandler = AnyResponseHandler { response, data -> ResponseHandlerResult<Bool> in
                let statusCode = HTTPStatus(rawValue: response.statusCode)

                switch statusCode {
                case let statusCode where statusCode.isSuccess:
                    return .success(true)

                case .notFound:
                    return .success(false)

                default:
                    return .unhandledResponse(
                        try? self.responseDecoder.decode(
                            ServerErrorResponse.self,
                            from: data
                        )
                    )
                }
            }

            return addOperation(
                name: "delete-device",
                retryStrategy: retryStrategy,
                requestHandler: requestHandler,
                responseHandler: responseHandler,
                completionHandler: completion
            )
        }

        /// Rotate device key
        func rotateDeviceKey(
            accountNumber: String,
            identifier: String,
            publicKey: PublicKey,
            retryStrategy: REST.RetryStrategy,
            completion: @escaping CompletionHandler<Device>
        ) -> Cancellable {
            let requestHandler = AnyRequestHandler(
                createURLRequest: { endpoint, authorization in
                    let urlEncodedIdentifier = identifier
                        .addingPercentEncoding(withAllowedCharacters: .alphanumerics)!
                    let path = "devices/\(urlEncodedIdentifier)/pubkey"

                    var requestBuilder = self.requestFactory
                        .createURLRequestBuilder(
                            endpoint: endpoint,
                            method: .put,
                            path: path
                        )

                    requestBuilder.setAuthorization(authorization)

                    let request = RotateDeviceKeyRequest(
                        publicKey: publicKey
                    )
                    try requestBuilder.setHTTPBody(value: request)

                    return requestBuilder.getURLRequest()
                },
                requestAuthorization: { completion in
                    return self.configuration.accessTokenManager
                        .getAccessToken(
                            accountNumber: accountNumber,
                            retryStrategy: retryStrategy
                        ) { operationCompletion in
                            completion(operationCompletion.map { tokenData in
                                return .accessToken(tokenData.accessToken)
                            })
                        }
                }
            )

            let responseHandler = REST.defaultResponseHandler(
                decoding: Device.self,
                with: responseDecoder
            )

            return addOperation(
                name: "rotate-device-key",
                retryStrategy: retryStrategy,
                requestHandler: requestHandler,
                responseHandler: responseHandler,
                completionHandler: completion
            )
        }

    }

    struct CreateDeviceRequest: Encodable {
        let publicKey: PublicKey
        let hijackDNS: Bool

        private enum CodingKeys: String, CodingKey {
            case hijackDNS = "hijackDns"
            case publicKey = "pubkey"
        }

        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)

            try container.encode(publicKey.base64Key, forKey: .publicKey)
            try container.encode(hijackDNS, forKey: .hijackDNS)
        }
    }

    fileprivate struct RotateDeviceKeyRequest: Encodable {
        let publicKey: PublicKey

        private enum CodingKeys: String, CodingKey {
            case publicKey = "pubkey"
        }

        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)

            try container.encode(publicKey.base64Key, forKey: .publicKey)
        }
    }

    struct Device: Decodable {
        let id: String
        let name: String
        let pubkey: Data
        let hijackDNS: Bool
        let created: Date
        let ipv4Address: IPAddressRange
        let ipv6Address: IPAddressRange
        let ports: [Port]

        private enum CodingKeys: String, CodingKey {
            case hijackDNS = "hijackDns"
            case id, name, pubkey, created, ipv4Address, ipv6Address, ports
        }
    }

    struct Port: Decodable {
        let id: String
    }

}
