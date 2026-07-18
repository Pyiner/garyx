import XCTest
@testable import GaryxMobileCore

final class GaryxChatImageUploadBatchTests: XCTestCase {
    func testMultipleImagesBecomeOneBatchRequestInSelectionOrder() {
        let images = [
            GaryxMobileSelectedImage(
                name: "photo-1.jpg",
                mediaType: "image/jpeg",
                data: Data([0x01, 0x02])
            ),
            GaryxMobileSelectedImage(
                name: "photo-2.png",
                mediaType: "image/png",
                data: Data([0x03, 0x04, 0x05])
            ),
            GaryxMobileSelectedImage(
                name: "photo-3.heic",
                mediaType: "image/heic",
                data: Data([0x06])
            ),
        ]

        let batch = GaryxChatImageUploadBatch.prepare(images)

        XCTAssertEqual(batch.request.files.count, images.count)
        XCTAssertEqual(batch.request.files.map(\.name), images.map(\.name))
        XCTAssertEqual(batch.request.files.map(\.mediaType), images.map { Optional($0.mediaType) })
        XCTAssertEqual(
            batch.request.files.map(\.dataBase64),
            images.map { $0.data.base64EncodedString() }
        )
    }

    func testUploadedBatchMapsToPayloadItemsWithLocalPreviews() throws {
        let images = [
            GaryxMobileSelectedImage(
                name: "photo-1.jpg",
                mediaType: "image/jpeg",
                data: Data([0x01, 0x02])
            ),
            GaryxMobileSelectedImage(
                name: "photo-2.png",
                mediaType: "image/png",
                data: Data([0x03, 0x04])
            ),
        ]
        let batch = GaryxChatImageUploadBatch.prepare(images)
        let uploaded = try JSONDecoder().decode(
            GaryxUploadChatAttachmentsResult.self,
            from: Data(
                """
                {
                  "files": [
                    {
                      "kind": "image",
                      "path": "/tmp/upload-1.jpg",
                      "name": "photo-1.jpg",
                      "mediaType": "image/jpeg"
                    },
                    {
                      "kind": "image",
                      "path": "/tmp/upload-2.png",
                      "name": "photo-2.png",
                      "mediaType": "image/png"
                    }
                  ]
                }
                """.utf8
            )
        )

        let attachments = try XCTUnwrap(
            batch.composerPayloadItems(from: uploaded.files) { index, _ in
                "attachment-\(index)"
            }
        )

        XCTAssertEqual(attachments.map(\.id), ["attachment-0", "attachment-1"])
        XCTAssertEqual(attachments.map(\.path), ["/tmp/upload-1.jpg", "/tmp/upload-2.png"])
        XCTAssertEqual(
            attachments.map(\.previewDataUrl),
            [
                "data:image/jpeg;base64,\(Data([0x01, 0x02]).base64EncodedString())",
                "data:image/png;base64,\(Data([0x03, 0x04]).base64EncodedString())",
            ]
        )
    }

    func testPartialGatewayResponseIsRejectedInsteadOfDroppingAnImage() throws {
        let batch = GaryxChatImageUploadBatch.prepare([
            GaryxMobileSelectedImage(name: "photo-1.jpg", mediaType: "image/jpeg", data: Data([0x01])),
            GaryxMobileSelectedImage(name: "photo-2.jpg", mediaType: "image/jpeg", data: Data([0x02])),
        ])
        let uploaded = try JSONDecoder().decode(
            GaryxUploadChatAttachmentsResult.self,
            from: Data(
                """
                {
                  "files": [
                    {
                      "kind": "image",
                      "path": "/tmp/upload-1.jpg",
                      "name": "photo-1.jpg",
                      "mediaType": "image/jpeg"
                    }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertNil(batch.composerPayloadItems(from: uploaded.files))
    }
}
