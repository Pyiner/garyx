import Foundation
import Photos

enum GaryxImagePhotoLibraryError: Error {
    case addPermissionDenied
    case sourceUnavailable
    case writeFailed(Error?)
}

enum GaryxImagePhotoLibrary {
    static func save(
        source: GaryxImagePreviewSource,
        loadGatewayDataURL: ((String) async -> String?)?
    ) async throws {
        try await requireAddOnlyAuthorization()
        let payload: GaryxImageSavePayload
        do {
            payload = try await GaryxImageSaveLoader.load(
                source.saveRequest,
                readFile: readFile,
                resolveGatewayDataURL: { path in
                    guard let loadGatewayDataURL else { return nil }
                    return await loadGatewayDataURL(path)
                },
                fetchRemote: fetchRemote
            )
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw GaryxImagePhotoLibraryError.sourceUnavailable
        }
        try Task.checkCancellation()
        try await addToPhotoLibrary(payload)
    }

    private static func requireAddOnlyAuthorization() async throws {
        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        switch status {
        case .authorized, .limited:
            return
        case .denied, .restricted, .notDetermined:
            throw GaryxImagePhotoLibraryError.addPermissionDenied
        @unknown default:
            throw GaryxImagePhotoLibraryError.addPermissionDenied
        }
    }

    private static func readFile(_ path: String) async throws -> Data {
        try await Task.detached(priority: .userInitiated) {
            try Data(contentsOf: URL(fileURLWithPath: path), options: .mappedIfSafe)
        }.value
    }

    private static func fetchRemote(_ rawURL: String) async throws -> GaryxImageSaveRemoteResource {
        guard let url = URL(string: rawURL),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https" else {
            throw GaryxImagePhotoLibraryError.sourceUnavailable
        }
        let (data, response) = try await URLSession.shared.data(from: url)
        if let http = response as? HTTPURLResponse, !(200...299).contains(http.statusCode) {
            throw GaryxImagePhotoLibraryError.sourceUnavailable
        }
        return GaryxImageSaveRemoteResource(
            data: data,
            mimeType: response.mimeType,
            suggestedFilename: response.suggestedFilename
        )
    }

    private static func addToPhotoLibrary(_ payload: GaryxImageSavePayload) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            PHPhotoLibrary.shared().performChanges {
                let options = PHAssetResourceCreationOptions()
                options.originalFilename = payload.originalFilename
                options.uniformTypeIdentifier = payload.uniformTypeIdentifier
                PHAssetCreationRequest.forAsset().addResource(
                    with: .photo,
                    data: payload.data,
                    options: options
                )
            } completionHandler: { succeeded, error in
                if succeeded {
                    continuation.resume()
                } else {
                    continuation.resume(throwing: GaryxImagePhotoLibraryError.writeFailed(error))
                }
            }
        }
    }
}

private extension GaryxImagePreviewSource {
    var saveRequest: GaryxImageSaveRequest {
        GaryxImageSaveRequest(
            title: displayTitle,
            dataURL: dataUrl,
            filePath: filePath,
            gatewayFilePath: gatewayFilePath,
            remoteURL: remoteUrl
        )
    }
}
