---
description: DigitalOcean Spaces S3 API compatibility facts — supported in-region CopyObject and UploadPartCopy, 5 GiB single-copy ceiling, ListObjectsV2, multipart uploads, and incompatibility with aws-chunked streaming-trailer request checksums.
---

# DigitalOcean Spaces — S3 Compatibility

## Supported object operations

- **`PutObject`, `HeadObject`, `GetObject`, `DeleteObject`:** Supported core
  object operations.
- **Multipart uploads:** `CreateMultipartUpload`, `UploadPart`,
  `CompleteMultipartUpload`, `AbortMultipartUpload`, and `ListMultipartUploads`
  are supported for large objects.
- **`ListObjectsV2`:** Supported.
- **`CopyObject`:** Supported for in-region, same-bucket/same-cluster copies up
  to the 5 GiB S3 single-object copy limit. Cross-region and cross-cluster
  copies are not supported.
- **`UploadPartCopy`:** Supported for in-region multipart server-side copies
  above the 5 GiB single-copy threshold. Cross-region and cross-cluster copies
  are not supported.

## Request checksum compatibility

- DigitalOcean Spaces does not support the `aws-chunked`
  `STREAMING-UNSIGNED-PAYLOAD-TRAILER` CRC32 request-checksum framing emitted
  by default in newer AWS SDKs and may reject or mis-store objects uploaded
  with it.
- Clients targeting Spaces must disable default request-checksum calculation
  (for example `RequestChecksumCalculation::WhenRequired` in `aws-sdk-s3` or
  `request_checksum_calculation="when_required"` in `botocore`).

## Sources

- https://docs.digitalocean.com/reference/api/spaces/
- https://docs.digitalocean.com/products/spaces/reference/s3-compatibility/
- https://docs.digitalocean.com/products/spaces/how-to/use-aws-sdks/
- https://docs.aws.amazon.com/AmazonS3/latest/API/API_CopyObject.html
