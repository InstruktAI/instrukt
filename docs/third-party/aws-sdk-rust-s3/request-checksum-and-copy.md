---
description: AWS SDK for Rust S3 client request-checksum behavior (RequestChecksumCalculation::WhenRequired to avoid aws-chunked streaming-trailer framing on S3-compatible providers) and server-side copy limits (5 GiB CopyObject ceiling vs multipart UploadPartCopy).
---

# aws-sdk-s3 (AWS SDK for Rust) — Request Checksums and Server-Side Copy

## Default request checksum calculation

- Recent AWS SDK releases default `RequestChecksumCalculation` to
  `WhenSupported` for S3 upload operations (`PutObject`, `UploadPart`).
- Under `WhenSupported`, the SDK sends `Content-Encoding: aws-chunked`,
  `x-amz-content-sha256: STREAMING-UNSIGNED-PAYLOAD-TRAILER`,
  `x-amz-trailer: x-amz-checksum-crc32`, and
  `x-amz-sdk-checksum-algorithm: CRC32`, wrapping the request body in
  chunked-trailer framing rather than a plain fixed-length payload.
- Many non-AWS S3-compatible object stores either reject those headers or store
  the chunk-framed payload non-verbatim, causing persisted object lengths to
  diverge from the uploaded bytes.
- Configuring `RequestChecksumCalculation::WhenRequired` on the S3 client
  builder suppresses default streaming-trailer checksums for operations that do
  not require a checksum, sending plain payloads with exact `Content-Length`
  suitable for S3-compatible providers.

## Server-side copy size limit

- Standard S3 `CopyObject` is limited to objects up to 5 GiB in a single
  request.
- Objects exceeding 5 GiB require multipart server-side copy:
  `CreateMultipartUpload`, sequential `UploadPartCopy` calls specifying
  `copy_source_range("bytes=<start>-<end>")` (part sizes between 5 MiB and
  5 GiB, up to 10,000 parts), and `CompleteMultipartUpload` (with
  `AbortMultipartUpload` on failure).

## Sources

- https://docs.aws.amazon.com/sdk-for-rust/latest/dg/welcome.html
- https://docs.rs/aws-sdk-s3/latest/aws_sdk_s3/config/enum.RequestChecksumCalculation.html
- https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html
- https://docs.aws.amazon.com/AmazonS3/latest/API/API_UploadPartCopy.html
