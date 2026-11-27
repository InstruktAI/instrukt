//! Provider-neutral S3-compatible object storage adapter for home backup
//! (`project/instrukt/spec/feature/runner/home-backup`).
//!
//! Encapsulates `aws-sdk-s3` behind a synchronous facade reusing
//! [`instrukt_outside::runtime::block_on`] with `RequestChecksumCalculation::WhenRequired`
//! (no `aws-chunked` streaming-trailer framing), path-style addressing, bounded
//! retries, and connect/read timeouts without an overall operation timeout.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::num::NonZeroU32;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, RequestChecksumCalculation};
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::primitives::{ByteStream, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_smithy_http_client::{Builder as HttpClientBuilder, tls};
use instrukt_core::config::BackupConfig;
use instrukt_core::fsx;
use instrukt_core::machine::MachineId;
use jiff::{SignedDuration, Timestamp};
use tokio::io::AsyncReadExt as _;
use zeroize::Zeroizing;

use super::_archive::ARCHIVE_EXTENSION;
use super::_state::redact_secrets;
use super::BackupError;

/// Age after which orphaned `_staging/` objects and incomplete multipart uploads
/// for this machine are cleaned up during post-validation maintenance.
pub const ORPHAN_CLEANUP_AGE: SignedDuration = SignedDuration::from_hours(24);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_SINGLE_UPLOAD_MAX: u64 = 64 * 1024 * 1024;
const DEFAULT_UPLOAD_PART_SIZE: u64 = 64 * 1024 * 1024;
const DEFAULT_SINGLE_COPY_MAX: u64 = 4 * 1024 * 1024 * 1024;
const DEFAULT_COPY_PART_SIZE: u64 = 512 * 1024 * 1024;

/// Size thresholds governing single-request vs multipart upload and server-side
/// copy. Production uses [`TransferThresholds::default`]; unit tests inject
/// smaller thresholds directly via [`S3Storage::with_thresholds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferThresholds {
    pub single_upload_max: u64,
    pub upload_part_size: u64,
    pub single_copy_max: u64,
    pub copy_part_size: u64,
}

impl Default for TransferThresholds {
    fn default() -> Self {
        Self {
            single_upload_max: DEFAULT_SINGLE_UPLOAD_MAX,
            upload_part_size: DEFAULT_UPLOAD_PART_SIZE,
            single_copy_max: DEFAULT_SINGLE_COPY_MAX,
            copy_part_size: DEFAULT_COPY_PART_SIZE,
        }
    }
}

/// One backup generation present in remote object storage for a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupGeneration {
    pub key: String,
    pub filename: String,
    pub size_bytes: u64,
    pub timestamp: Option<Timestamp>,
    pub last_modified: Option<Timestamp>,
    pub validated: bool,
}

impl BackupGeneration {
    fn sort_timestamp(&self) -> Option<Timestamp> {
        self.timestamp.or(self.last_modified)
    }
}

/// Summary of retention pruning and stale orphan cleanup for one machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub pruned_generations: Vec<String>,
    pub deleted_orphans: Vec<String>,
    pub aborted_uploads: Vec<String>,
}

/// Synchronous facade over an `aws-sdk-s3` client configured for one backup
/// target.
pub struct S3Storage {
    client: Client,
    bucket: String,
    prefix: Option<String>,
    thresholds: TransferThresholds,
    secrets: Vec<Zeroizing<String>>,
}

impl fmt::Debug for S3Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Storage")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("thresholds", &self.thresholds)
            .field("secrets", &"<redacted>")
            .finish()
    }
}

impl S3Storage {
    /// Constructs a production storage client with default multipart thresholds.
    pub fn new(config: &BackupConfig) -> Self {
        Self::with_thresholds(config, TransferThresholds::default())
    }

    /// Constructs a storage client with explicit multipart/copy thresholds.
    pub fn with_thresholds(config: &BackupConfig, thresholds: TransferThresholds) -> Self {
        let http_client = HttpClientBuilder::new()
            .tls_provider(tls::Provider::Rustls(
                tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        let credentials = Credentials::new(
            config.access_key_id.expose(),
            config.secret_access_key.expose(),
            None,
            None,
            "instrukt-home-backup",
        );
        let timeout_config = TimeoutConfig::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build();
        let sdk_config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .http_client(http_client)
            .endpoint_url(config.endpoint.as_str())
            .region(Region::new(config.region.clone()))
            .force_path_style(true)
            .credentials_provider(credentials)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .retry_config(RetryConfig::standard())
            .timeout_config(timeout_config)
            .build();
        let client = Client::from_conf(sdk_config);
        let secrets = vec![
            Zeroizing::new(config.access_key_id.expose().to_owned()),
            Zeroizing::new(config.secret_access_key.expose().to_owned()),
        ];
        Self {
            client,
            bucket: config.bucket.clone(),
            prefix: config.prefix.clone(),
            thresholds,
            secrets,
        }
    }

    fn scrub(&self, text: &str) -> String {
        let refs: Vec<&str> = self.secrets.iter().map(|s| s.as_str()).collect();
        redact_secrets(text, &refs)
    }

    fn format_sdk_error<E>(&self, action: &str, key: &str, err: &E) -> BackupError
    where
        E: std::error::Error + 'static,
    {
        let raw = format!("{action} `{key}` failed: {}", DisplayErrorContext(err));
        BackupError::Storage(self.scrub(&raw))
    }

    fn run_async<T: Send>(
        &self,
        fut: impl std::future::Future<Output = Result<T, BackupError>> + Send,
    ) -> Result<T, BackupError> {
        instrukt_outside::runtime::block_on(fut)
            .map_err(|err| BackupError::Storage(format!("async runtime unavailable: {err}")))?
    }

    /// Uploads a local encrypted archive to `staging_key`, switching from
    /// single `PutObject` to multipart upload above `single_upload_max`.
    pub fn upload_to_staging(
        &self,
        local_path: &Path,
        local_size: u64,
        staging_key: &str,
    ) -> Result<(), BackupError> {
        let bucket = self.bucket.clone();
        let key = staging_key.to_owned();
        let path = local_path.to_path_buf();
        let thresholds = self.thresholds;
        let client = self.client.clone();

        self.run_async(async move {
            if local_size <= thresholds.single_upload_max {
                let content_length = i64::try_from(local_size).map_err(|_| {
                    BackupError::Storage(format!(
                        "archive `{key}` exceeds S3 Content-Length range ({local_size} bytes)"
                    ))
                })?;
                let body = ByteStream::read_from()
                    .path(&path)
                    .build()
                    .await
                    .map_err(|err| {
                        BackupError::Storage(self.scrub(&format!(
                            "cannot open staged archive `{}` for upload: {err}",
                            path.display()
                        )))
                    })?;
                client
                    .put_object()
                    .bucket(&bucket)
                    .key(&key)
                    .content_length(content_length)
                    .body(body)
                    .send()
                    .await
                    .map_err(|err| self.format_sdk_error("PutObject", &key, &err))?;
                return Ok(());
            }

            let part_size = thresholds.upload_part_size.max(1);
            let created = client
                .create_multipart_upload()
                .bucket(&bucket)
                .key(&key)
                .send()
                .await
                .map_err(|err| self.format_sdk_error("CreateMultipartUpload", &key, &err))?;
            let upload_id = created.upload_id().map(str::to_owned).ok_or_else(|| {
                BackupError::Storage(format!(
                    "CreateMultipartUpload `{key}` returned no upload id"
                ))
            })?;

            let upload_result: Result<(), BackupError> = async {
                let mut completed_parts = Vec::new();
                let mut offset = 0u64;
                let mut part_number = 1i32;
                while offset < local_size {
                    let remaining = local_size - offset;
                    let len = remaining.min(part_size);
                    let content_length = i64::try_from(len).map_err(|_| {
                        BackupError::Storage(format!("part {part_number} exceeds S3 length range"))
                    })?;
                    let body = ByteStream::read_from()
                        .path(&path)
                        .offset(offset)
                        .length(Length::Exact(len))
                        .build()
                        .await
                        .map_err(|err| {
                            BackupError::Storage(self.scrub(&format!(
                                "cannot read part {part_number} of `{}`: {err}",
                                path.display()
                            )))
                        })?;
                    let uploaded = client
                        .upload_part()
                        .bucket(&bucket)
                        .key(&key)
                        .upload_id(&upload_id)
                        .part_number(part_number)
                        .content_length(content_length)
                        .body(body)
                        .send()
                        .await
                        .map_err(|err| self.format_sdk_error("UploadPart", &key, &err))?;
                    completed_parts.push(
                        CompletedPart::builder()
                            .part_number(part_number)
                            .set_e_tag(uploaded.e_tag().map(str::to_owned))
                            .build(),
                    );
                    offset += len;
                    part_number = part_number.checked_add(1).ok_or_else(|| {
                        BackupError::Storage(format!(
                            "multipart upload `{key}` exceeded part count"
                        ))
                    })?;
                }
                let completed = CompletedMultipartUpload::builder()
                    .set_parts(Some(completed_parts))
                    .build();
                client
                    .complete_multipart_upload()
                    .bucket(&bucket)
                    .key(&key)
                    .upload_id(&upload_id)
                    .multipart_upload(completed)
                    .send()
                    .await
                    .map_err(|err| self.format_sdk_error("CompleteMultipartUpload", &key, &err))?;
                Ok(())
            }
            .await;

            if let Err(err) = upload_result {
                let _ = client
                    .abort_multipart_upload()
                    .bucket(&bucket)
                    .key(&key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                return Err(err);
            }
            Ok(())
        })
    }

    /// Reads object byte length via `HeadObject`.
    pub fn head_object(&self, key: &str) -> Result<u64, BackupError> {
        let bucket = self.bucket.clone();
        let key_owned = key.to_owned();
        let client = self.client.clone();
        self.run_async(async move {
            let head = client
                .head_object()
                .bucket(&bucket)
                .key(&key_owned)
                .send()
                .await
                .map_err(|err| self.format_sdk_error("HeadObject", &key_owned, &err))?;
            let length = head.content_length().ok_or_else(|| {
                BackupError::Storage(format!(
                    "HeadObject `{key_owned}` did not return Content-Length"
                ))
            })?;
            u64::try_from(length).map_err(|_| {
                BackupError::Storage(format!(
                    "HeadObject `{key_owned}` returned negative Content-Length ({length})"
                ))
            })
        })
    }

    /// Verifies `staging_key` matches `expected_size` via `HeadObject`. On
    /// mismatch or error, performs best-effort deletion of `staging_key` and
    /// refuses publication.
    pub fn verify_staging(&self, staging_key: &str, expected_size: u64) -> Result<(), BackupError> {
        match self.head_object(staging_key) {
            Ok(remote_size) if remote_size == expected_size => Ok(()),
            Ok(remote_size) => {
                let _ = self.delete_object(staging_key);
                Err(BackupError::Storage(format!(
                    "staged object `{staging_key}` size mismatch: remote has {remote_size} bytes, expected {expected_size} bytes"
                )))
            }
            Err(err) => {
                let _ = self.delete_object(staging_key);
                Err(err)
            }
        }
    }

    /// Promotes `staging_key` to `live_key` via server-side copy, switching to
    /// multipart `UploadPartCopy` above `single_copy_max`. On any error,
    /// performs best-effort deletion of `staging_key`.
    pub fn promote_staging(
        &self,
        staging_key: &str,
        live_key: &str,
        size_bytes: u64,
    ) -> Result<(), BackupError> {
        let result = self.promote_staging_inner(staging_key, live_key, size_bytes);
        if result.is_err() {
            let _ = self.delete_object(staging_key);
        }
        result
    }

    fn promote_staging_inner(
        &self,
        staging_key: &str,
        live_key: &str,
        size_bytes: u64,
    ) -> Result<(), BackupError> {
        let bucket = self.bucket.clone();
        let copy_source = format!(
            "{}/{}",
            percent_encode_segment(&self.bucket),
            percent_encode_key(staging_key)
        );
        let live_key_owned = live_key.to_owned();
        let thresholds = self.thresholds;
        let client = self.client.clone();

        self.run_async(async move {
            if size_bytes <= thresholds.single_copy_max {
                client
                    .copy_object()
                    .bucket(&bucket)
                    .key(&live_key_owned)
                    .copy_source(&copy_source)
                    .send()
                    .await
                    .map_err(|err| self.format_sdk_error("CopyObject", &live_key_owned, &err))?;
                return Ok(());
            }

            let part_size = thresholds.copy_part_size.max(1);
            let created = client
                .create_multipart_upload()
                .bucket(&bucket)
                .key(&live_key_owned)
                .send()
                .await
                .map_err(|err| {
                    self.format_sdk_error("CreateMultipartUpload", &live_key_owned, &err)
                })?;
            let upload_id = created.upload_id().map(str::to_owned).ok_or_else(|| {
                BackupError::Storage(format!(
                    "CreateMultipartUpload `{live_key_owned}` returned no upload id"
                ))
            })?;

            let copy_result: Result<(), BackupError> = async {
                let mut completed_parts = Vec::new();
                let mut offset = 0u64;
                let mut part_number = 1i32;
                while offset < size_bytes {
                    let end = offset
                        .saturating_add(part_size)
                        .saturating_sub(1)
                        .min(size_bytes.saturating_sub(1));
                    let range = format!("bytes={offset}-{end}");
                    let copied = client
                        .upload_part_copy()
                        .bucket(&bucket)
                        .key(&live_key_owned)
                        .upload_id(&upload_id)
                        .part_number(part_number)
                        .copy_source(&copy_source)
                        .copy_source_range(range)
                        .send()
                        .await
                        .map_err(|err| {
                            self.format_sdk_error("UploadPartCopy", &live_key_owned, &err)
                        })?;
                    let etag = copied
                        .copy_part_result()
                        .and_then(|res| res.e_tag())
                        .map(str::to_owned);
                    completed_parts.push(
                        CompletedPart::builder()
                            .part_number(part_number)
                            .set_e_tag(etag)
                            .build(),
                    );
                    offset = end.saturating_add(1);
                    part_number = part_number.checked_add(1).ok_or_else(|| {
                        BackupError::Storage(format!(
                            "multipart copy `{live_key_owned}` exceeded part count"
                        ))
                    })?;
                }
                let completed = CompletedMultipartUpload::builder()
                    .set_parts(Some(completed_parts))
                    .build();
                client
                    .complete_multipart_upload()
                    .bucket(&bucket)
                    .key(&live_key_owned)
                    .upload_id(&upload_id)
                    .multipart_upload(completed)
                    .send()
                    .await
                    .map_err(|err| {
                        self.format_sdk_error("CompleteMultipartUpload", &live_key_owned, &err)
                    })?;
                Ok(())
            }
            .await;

            if let Err(err) = copy_result {
                let _ = client
                    .abort_multipart_upload()
                    .bucket(&bucket)
                    .key(&live_key_owned)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                return Err(err);
            }
            Ok(())
        })
    }

    /// Writes the empty `_validated/<live_key>` marker object.
    pub fn write_validation_marker(&self, live_key: &str) -> Result<(), BackupError> {
        let marker_key = validated_key(live_key);
        let bucket = self.bucket.clone();
        let client = self.client.clone();
        self.run_async(async move {
            client
                .put_object()
                .bucket(&bucket)
                .key(&marker_key)
                .content_length(0)
                .body(ByteStream::from_static(b""))
                .send()
                .await
                .map_err(|err| self.format_sdk_error("PutObject", &marker_key, &err))?;
            Ok(())
        })
    }

    /// Deletes one object key.
    pub fn delete_object(&self, key: &str) -> Result<(), BackupError> {
        let bucket = self.bucket.clone();
        let key_owned = key.to_owned();
        let client = self.client.clone();
        self.run_async(async move {
            client
                .delete_object()
                .bucket(&bucket)
                .key(&key_owned)
                .send()
                .await
                .map_err(|err| self.format_sdk_error("DeleteObject", &key_owned, &err))?;
            Ok(())
        })
    }

    /// Streams `key` directly to `dest_path` (`create_new`, `0600`) without
    /// buffering the whole object in memory.
    pub fn download_object(&self, key: &str, dest_path: &Path) -> Result<u64, BackupError> {
        let bucket = self.bucket.clone();
        let key_owned = key.to_owned();
        let dest = dest_path.to_path_buf();
        let client = self.client.clone();
        self.run_async(async move {
            let output = client
                .get_object()
                .bucket(&bucket)
                .key(&key_owned)
                .send()
                .await
                .map_err(|err| self.format_sdk_error("GetObject", &key_owned, &err))?;
            let mut open_opts = OpenOptions::new();
            open_opts.create_new(true).write(true).mode(0o600);
            let mut file = fsx::open(&open_opts, &dest).map_err(|source| BackupError::Io {
                path: dest.clone(),
                source,
            })?;
            let mut reader = output.body.into_async_read();
            let mut buf = [0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                let read = reader.read(&mut buf).await.map_err(|err| {
                    BackupError::Storage(
                        self.scrub(&format!("streaming GetObject `{key_owned}` failed: {err}")),
                    )
                })?;
                if read == 0 {
                    break;
                }
                file.write_all(&buf[..read])
                    .map_err(|source| BackupError::Io {
                        path: dest.clone(),
                        source,
                    })?;
                total += read as u64;
            }
            file.sync_all().map_err(|source| BackupError::Io {
                path: dest.clone(),
                source,
            })?;
            Ok(total)
        })
    }

    /// Lists all generations in `machine_id`'s namespace, newest first,
    /// distinguishing validated from unvalidated generations.
    pub fn list_generations(
        &self,
        machine_id: &MachineId,
    ) -> Result<Vec<BackupGeneration>, BackupError> {
        let live_prefix = live_namespace_prefix(self.prefix.as_deref(), machine_id);
        let validated_prefix = validated_namespace_prefix(self.prefix.as_deref(), machine_id);

        let live_objects = self.list_objects_with_prefix(&live_prefix)?;
        let validated_objects = self.list_objects_with_prefix(&validated_prefix)?;

        let validated_keys: BTreeSet<String> = validated_objects
            .into_iter()
            .filter_map(|obj| obj.key.strip_prefix("_validated/").map(str::to_owned))
            .collect();

        let mut generations: Vec<BackupGeneration> = live_objects
            .into_iter()
            .filter(|obj| obj.key.starts_with(&live_prefix))
            .filter_map(|obj| {
                let filename = obj.key.strip_prefix(&live_prefix)?;
                if filename.is_empty() || filename.contains('/') {
                    return None;
                }
                let validated = validated_keys.contains(&obj.key);
                let timestamp = parse_filename_timestamp(filename);
                Some(BackupGeneration {
                    filename: filename.to_owned(),
                    key: obj.key,
                    size_bytes: obj.size_bytes,
                    timestamp,
                    last_modified: obj.last_modified,
                    validated,
                })
            })
            .collect();

        generations.sort_by(|a, b| {
            b.sort_timestamp()
                .cmp(&a.sort_timestamp())
                .then_with(|| b.key.cmp(&a.key))
        });
        Ok(generations)
    }

    /// Prunes excess generations beyond `keep` for `machine_id` (evicting
    /// unvalidated before validated, oldest first within each group, and
    /// deleting paired `_validated/<key>` markers) and removes `_staging/`
    /// objects and incomplete multipart uploads older than 24h for this machine.
    pub fn prune_and_clean_orphans(
        &self,
        machine_id: &MachineId,
        keep: NonZeroU32,
        now: Timestamp,
    ) -> Result<PruneReport, BackupError> {
        let mut report = PruneReport::default();
        let generations = self.list_generations(machine_id)?;
        let keep_count = keep.get() as usize;
        if generations.len() > keep_count {
            let excess = generations.len() - keep_count;
            let mut eviction_order = generations;
            eviction_order.sort_by(|a, b| {
                a.validated
                    .cmp(&b.validated)
                    .then_with(|| a.sort_timestamp().cmp(&b.sort_timestamp()))
                    .then_with(|| a.key.cmp(&b.key))
            });
            for victim in eviction_order.into_iter().take(excess) {
                self.delete_object(&victim.key)?;
                if victim.validated {
                    self.delete_object(&validated_key(&victim.key))?;
                }
                report.pruned_generations.push(victim.key);
            }
        }

        let cutoff = now
            .checked_sub(ORPHAN_CLEANUP_AGE)
            .unwrap_or(Timestamp::MIN);
        let staging_prefix = staging_namespace_prefix(self.prefix.as_deref(), machine_id);
        let live_prefix = live_namespace_prefix(self.prefix.as_deref(), machine_id);

        for obj in self.list_objects_with_prefix(&staging_prefix)? {
            if obj.key.starts_with(&staging_prefix)
                && obj.last_modified.is_some_and(|ts| ts < cutoff)
            {
                self.delete_object(&obj.key)?;
                report.deleted_orphans.push(obj.key);
            }
        }

        for prefix in [&staging_prefix, &live_prefix] {
            for upload in self.list_multipart_uploads_with_prefix(prefix)? {
                if upload.key.starts_with(prefix) && upload.initiated.is_some_and(|ts| ts < cutoff)
                {
                    self.abort_multipart_upload(&upload.key, &upload.upload_id)?;
                    report.aborted_uploads.push(upload.key);
                }
            }
        }

        Ok(report)
    }

    fn list_objects_with_prefix(&self, prefix: &str) -> Result<Vec<ListedObject>, BackupError> {
        let bucket = self.bucket.clone();
        let prefix_owned = prefix.to_owned();
        let client = self.client.clone();
        self.run_async(async move {
            let mut out = Vec::new();
            let mut token: Option<String> = None;
            loop {
                let mut req = client
                    .list_objects_v2()
                    .bucket(&bucket)
                    .prefix(&prefix_owned);
                if let Some(ref continuation) = token {
                    req = req.continuation_token(continuation);
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|err| self.format_sdk_error("ListObjectsV2", &prefix_owned, &err))?;
                for obj in resp.contents() {
                    if let Some(key) = obj.key() {
                        let size_bytes =
                            obj.size().and_then(|n| u64::try_from(n).ok()).unwrap_or(0);
                        let last_modified = obj.last_modified().and_then(smithy_to_timestamp);
                        out.push(ListedObject {
                            key: key.to_owned(),
                            size_bytes,
                            last_modified,
                        });
                    }
                }
                if resp.is_truncated() == Some(true) {
                    token = resp.next_continuation_token().map(str::to_owned);
                    if token.is_none() {
                        break;
                    }
                } else {
                    break;
                }
            }
            Ok(out)
        })
    }

    fn list_multipart_uploads_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<ListedUpload>, BackupError> {
        let bucket = self.bucket.clone();
        let prefix_owned = prefix.to_owned();
        let client = self.client.clone();
        self.run_async(async move {
            let mut out = Vec::new();
            let mut key_marker: Option<String> = None;
            let mut upload_id_marker: Option<String> = None;
            loop {
                let mut req = client
                    .list_multipart_uploads()
                    .bucket(&bucket)
                    .prefix(&prefix_owned);
                if let Some(ref km) = key_marker {
                    req = req.key_marker(km);
                }
                if let Some(ref um) = upload_id_marker {
                    req = req.upload_id_marker(um);
                }
                let resp = req.send().await.map_err(|err| {
                    self.format_sdk_error("ListMultipartUploads", &prefix_owned, &err)
                })?;
                for upload in resp.uploads() {
                    if let (Some(key), Some(upload_id)) = (upload.key(), upload.upload_id()) {
                        out.push(ListedUpload {
                            key: key.to_owned(),
                            upload_id: upload_id.to_owned(),
                            initiated: upload.initiated().and_then(smithy_to_timestamp),
                        });
                    }
                }
                if resp.is_truncated() == Some(true) {
                    key_marker = resp.next_key_marker().map(str::to_owned);
                    upload_id_marker = resp.next_upload_id_marker().map(str::to_owned);
                    if key_marker.is_none() && upload_id_marker.is_none() {
                        break;
                    }
                } else {
                    break;
                }
            }
            Ok(out)
        })
    }

    fn abort_multipart_upload(&self, key: &str, upload_id: &str) -> Result<(), BackupError> {
        let bucket = self.bucket.clone();
        let key_owned = key.to_owned();
        let upload_id_owned = upload_id.to_owned();
        let client = self.client.clone();
        self.run_async(async move {
            client
                .abort_multipart_upload()
                .bucket(&bucket)
                .key(&key_owned)
                .upload_id(&upload_id_owned)
                .send()
                .await
                .map_err(|err| self.format_sdk_error("AbortMultipartUpload", &key_owned, &err))?;
            Ok(())
        })
    }
}

struct ListedObject {
    key: String,
    size_bytes: u64,
    last_modified: Option<Timestamp>,
}

struct ListedUpload {
    key: String,
    upload_id: String,
    initiated: Option<Timestamp>,
}

fn smithy_to_timestamp(dt: &aws_sdk_s3::primitives::DateTime) -> Option<Timestamp> {
    let nanos = i32::try_from(dt.subsec_nanos()).ok()?;
    Timestamp::new(dt.secs(), nanos).ok()
}

/// Namespace for one machine: `<prefix>/<machine-id>` or `<machine-id>`.
pub(super) fn machine_namespace(prefix: Option<&str>, machine_id: &MachineId) -> String {
    match prefix
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty())
    {
        Some(p) => format!("{p}/{}", machine_id.as_str()),
        None => machine_id.as_str().to_owned(),
    }
}

pub(super) fn live_namespace_prefix(prefix: Option<&str>, machine_id: &MachineId) -> String {
    format!("{}/", machine_namespace(prefix, machine_id))
}

pub(super) fn staging_namespace_prefix(prefix: Option<&str>, machine_id: &MachineId) -> String {
    format!("_staging/{}/", machine_namespace(prefix, machine_id))
}

pub(super) fn validated_namespace_prefix(prefix: Option<&str>, machine_id: &MachineId) -> String {
    format!("_validated/{}/", machine_namespace(prefix, machine_id))
}

pub(super) fn generation_key(
    prefix: Option<&str>,
    machine_id: &MachineId,
    filename: &str,
) -> String {
    format!("{}/{filename}", machine_namespace(prefix, machine_id))
}

pub(super) fn staging_key(live_key: &str) -> String {
    format!("_staging/{live_key}")
}

pub(super) fn validated_key(live_key: &str) -> String {
    format!("_validated/{live_key}")
}

/// Parses `instrukt-home.<YYYYMMDDTHHMMSSZ>.tar.zst.age` back into a UTC
/// [`Timestamp`].
pub(super) fn parse_filename_timestamp(filename: &str) -> Option<Timestamp> {
    let middle = filename
        .strip_prefix("instrukt-home.")?
        .strip_suffix(ARCHIVE_EXTENSION)?;
    let bytes = middle.as_bytes();
    if bytes.len() != 16 || bytes[8] != b'T' || bytes[15] != b'Z' {
        return None;
    }
    let iso = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &middle[0..4],
        &middle[4..6],
        &middle[6..8],
        &middle[9..11],
        &middle[11..13],
        &middle[13..15],
    );
    iso.parse().ok()
}

fn percent_encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_encode_key(key: &str) -> String {
    key.split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;
    use instrukt_core::config::{BackupTime, SecretValue};

    struct MockUpload {
        key: String,
        parts: BTreeMap<u32, Vec<u8>>,
        initiated: String,
    }

    #[derive(Default)]
    struct MockS3State {
        objects: BTreeMap<String, (Vec<u8>, String)>,
        uploads: HashMap<String, MockUpload>,
        next_upload_id: u64,
        forbidden_header_seen: Vec<String>,
        upload_part_count: usize,
        upload_part_copy_ranges: Vec<String>,
        fail_copy: bool,
        head_size_override: Option<usize>,
        aborted_uploads: Vec<(String, String)>,
        deleted_keys: Vec<String>,
    }

    struct MockS3 {
        endpoint: String,
        state: Arc<Mutex<MockS3State>>,
        server: Arc<tiny_http::Server>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl MockS3 {
        fn start() -> Self {
            let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
            let addr = server.server_addr().to_ip().unwrap();
            let endpoint = format!("http://127.0.0.1:{}", addr.port());
            let state = Arc::new(Mutex::new(MockS3State::default()));
            let srv = Arc::clone(&server);
            let st = Arc::clone(&state);
            let handle = thread::spawn(move || {
                for req in srv.incoming_requests() {
                    handle_mock_request(req, &st);
                }
            });
            Self {
                endpoint,
                state,
                server,
                handle: Some(handle),
            }
        }

        fn config(&self) -> BackupConfig {
            let identity = age::x25519::Identity::generate();
            BackupConfig {
                endpoint: url::Url::parse(&self.endpoint).unwrap(),
                region: "auto".into(),
                bucket: "test-bucket".into(),
                access_key_id: SecretValue::new("AKIA_MOCK_KEY".into()),
                secret_access_key: SecretValue::new("SECRET_MOCK_VAL".into()),
                age_recipient: identity.to_public(),
                prefix: Some("hosts".into()),
                keep: NonZeroU32::new(2).unwrap(),
                time: BackupTime::default(),
            }
        }
    }

    impl Drop for MockS3 {
        fn drop(&mut self) {
            self.server.unblock();
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn parse_query(url: &str) -> (&str, BTreeMap<String, String>) {
        let mut parts = url.splitn(2, '?');
        let path = parts.next().unwrap_or("");
        let mut query = BTreeMap::new();
        if let Some(q) = parts.next() {
            for (k, v) in form_urlencoded::parse(q.as_bytes()) {
                query.insert(k.into_owned(), v.into_owned());
            }
        }
        (path, query)
    }

    fn strip_bucket(path: &str) -> String {
        let trimmed = path.trim_start_matches('/');
        match trimmed.split_once('/') {
            Some((_, key)) => key.to_owned(),
            None => String::new(),
        }
    }

    fn xml_escape_text(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn handle_mock_request(mut req: tiny_http::Request, state: &Arc<Mutex<MockS3State>>) {
        let method = req.method().as_str().to_owned();
        let url = req.url().to_owned();
        let headers: HashMap<String, String> = req
            .headers()
            .iter()
            .map(|h| {
                (
                    h.field.as_str().as_str().to_ascii_lowercase(),
                    h.value.as_str().to_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        let _ = req.as_reader().read_to_end(&mut body);

        let (path, query) = parse_query(&url);
        let key = strip_bucket(path);

        let mut st = state.lock().unwrap();
        if matches!(method.as_str(), "PUT") && !headers.contains_key("x-amz-copy-source") {
            if headers
                .get("content-encoding")
                .is_some_and(|v| v.contains("aws-chunked"))
            {
                st.forbidden_header_seen
                    .push("content-encoding:aws-chunked".into());
            }
            if headers
                .get("x-amz-content-sha256")
                .is_some_and(|v| v == "STREAMING-UNSIGNED-PAYLOAD-TRAILER")
            {
                st.forbidden_header_seen
                    .push("x-amz-content-sha256:streaming-trailer".into());
            }
            for forbidden in ["x-amz-trailer", "x-amz-sdk-checksum-algorithm"] {
                if headers.contains_key(forbidden) {
                    st.forbidden_header_seen.push(forbidden.into());
                }
            }
        }

        match method.as_str() {
            "GET" if query.contains_key("uploads") => {
                let prefix = query.get("prefix").cloned().unwrap_or_default();
                let mut xml = String::from(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListMultipartUploadsResult>",
                );
                xml.push_str("<IsTruncated>false</IsTruncated>");
                for (upload_id, upload) in &st.uploads {
                    if upload.key.starts_with(&prefix) {
                        xml.push_str(&format!(
                            "<Upload><Key>{}</Key><UploadId>{}</UploadId><Initiated>{}</Initiated></Upload>",
                            xml_escape_text(&upload.key),
                            xml_escape_text(upload_id),
                            xml_escape_text(&upload.initiated),
                        ));
                    }
                }
                xml.push_str("</ListMultipartUploadsResult>");
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
            }
            "GET" if query.get("list-type").map(String::as_str) == Some("2") => {
                let prefix = query.get("prefix").cloned().unwrap_or_default();
                let mut xml =
                    String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult>");
                xml.push_str("<IsTruncated>false</IsTruncated>");
                for (okey, (bytes, modified)) in &st.objects {
                    if okey.starts_with(&prefix) {
                        xml.push_str(&format!(
                            "<Contents><Key>{}</Key><Size>{}</Size><LastModified>{}</LastModified></Contents>",
                            xml_escape_text(okey),
                            bytes.len(),
                            xml_escape_text(modified),
                        ));
                    }
                }
                xml.push_str("</ListBucketResult>");
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
            }
            "GET" => {
                if let Some((bytes, _)) = st.objects.get(&key).cloned() {
                    let _ =
                        req.respond(tiny_http::Response::from_data(bytes).with_status_code(200));
                } else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                }
            }
            "HEAD" => {
                if let Some((bytes, _)) = st.objects.get(&key) {
                    let len = st.head_size_override.unwrap_or(bytes.len());
                    let header =
                        tiny_http::Header::from_bytes("Content-Length", len.to_string()).unwrap();
                    let _ = req.respond(tiny_http::Response::empty(200).with_header(header));
                } else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                }
            }
            "POST" if query.contains_key("uploads") => {
                st.next_upload_id += 1;
                let upload_id = format!("u-{}", st.next_upload_id);
                st.uploads.insert(
                    upload_id.clone(),
                    MockUpload {
                        key: key.clone(),
                        parts: BTreeMap::new(),
                        initiated: "2030-01-06T03:00:00Z".into(),
                    },
                );
                let xml = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><InitiateMultipartUploadResult><UploadId>{upload_id}</UploadId></InitiateMultipartUploadResult>"
                );
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
            }
            "POST" if query.contains_key("uploadId") => {
                let upload_id = query.get("uploadId").cloned().unwrap_or_default();
                if let Some(upload) = st.uploads.remove(&upload_id) {
                    let mut assembled = Vec::new();
                    for (_, chunk) in upload.parts {
                        assembled.extend_from_slice(&chunk);
                    }
                    st.objects.insert(upload.key, (assembled, upload.initiated));
                    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUploadResult><ETag>\"done\"</ETag></CompleteMultipartUploadResult>";
                    let _ =
                        req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
                } else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                }
            }
            "PUT"
                if headers.contains_key("x-amz-copy-source")
                    && query.contains_key("partNumber") =>
            {
                let upload_id = query.get("uploadId").cloned().unwrap_or_default();
                let part_num: u32 = query
                    .get("partNumber")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let copy_src = headers
                    .get("x-amz-copy-source")
                    .cloned()
                    .unwrap_or_default();
                let src_key = strip_bucket(&copy_src);
                let range = headers
                    .get("x-amz-copy-source-range")
                    .cloned()
                    .unwrap_or_default();
                st.upload_part_copy_ranges.push(range.clone());
                let (start, end) = range
                    .strip_prefix("bytes=")
                    .and_then(|r| r.split_once('-'))
                    .map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()))
                    .unwrap();
                let slice = st.objects.get(&src_key).unwrap().0[start..=end].to_vec();
                st.uploads
                    .get_mut(&upload_id)
                    .unwrap()
                    .parts
                    .insert(part_num, slice);
                let xml = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyPartResult><ETag>\"cp-{part_num}\"</ETag></CopyPartResult>"
                );
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
            }
            "PUT" if headers.contains_key("x-amz-copy-source") => {
                if st.fail_copy {
                    let _ = req.respond(tiny_http::Response::empty(403));
                    return;
                }
                let copy_src = headers
                    .get("x-amz-copy-source")
                    .cloned()
                    .unwrap_or_default();
                let src_key = strip_bucket(&copy_src);
                if let Some(entry) = st.objects.get(&src_key).cloned() {
                    st.objects.insert(key, entry);
                    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyObjectResult><ETag>\"copied\"</ETag></CopyObjectResult>";
                    let _ =
                        req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
                } else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                }
            }
            "PUT" if query.contains_key("partNumber") => {
                let upload_id = query.get("uploadId").cloned().unwrap_or_default();
                let part_num: u32 = query
                    .get("partNumber")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                st.upload_part_count += 1;
                st.uploads
                    .get_mut(&upload_id)
                    .unwrap()
                    .parts
                    .insert(part_num, body);
                let etag =
                    tiny_http::Header::from_bytes("ETag", format!("\"p-{part_num}\"")).unwrap();
                let _ = req.respond(tiny_http::Response::empty(200).with_header(etag));
            }
            "PUT" => {
                st.objects
                    .insert(key, (body, "2030-01-06T03:00:00Z".into()));
                let etag = tiny_http::Header::from_bytes("ETag", "\"single\"").unwrap();
                let _ = req.respond(tiny_http::Response::empty(200).with_header(etag));
            }
            "DELETE" if query.contains_key("uploadId") => {
                let upload_id = query.get("uploadId").cloned().unwrap_or_default();
                st.uploads.remove(&upload_id);
                st.aborted_uploads.push((key, upload_id));
                let _ = req.respond(tiny_http::Response::empty(204));
            }
            "DELETE" => {
                st.objects.remove(&key);
                st.deleted_keys.push(key);
                let _ = req.respond(tiny_http::Response::empty(204));
            }
            _ => {
                let _ = req.respond(tiny_http::Response::empty(400));
            }
        }
    }

    #[test]
    fn key_formatting_and_timestamp_parsing() {
        let mid = MachineId::parse("0123456789abcdef").unwrap();
        let name = "instrukt-home.20300106T030000Z.tar.zst.age";
        assert_eq!(
            generation_key(Some("hosts"), &mid, name),
            "hosts/0123456789abcdef/instrukt-home.20300106T030000Z.tar.zst.age"
        );
        assert_eq!(
            generation_key(None, &mid, name),
            "0123456789abcdef/instrukt-home.20300106T030000Z.tar.zst.age"
        );
        assert_eq!(
            parse_filename_timestamp(name),
            Some("2030-01-06T03:00:00Z".parse().unwrap())
        );
    }

    #[test]
    fn multipart_upload_and_multipart_copy_work_without_aws_chunked_headers() {
        let mock = MockS3::start();
        let cfg = mock.config();
        let storage = S3Storage::with_thresholds(
            &cfg,
            TransferThresholds {
                single_upload_max: 64,
                upload_part_size: 40,
                single_copy_max: 64,
                copy_part_size: 50,
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let payload: Vec<u8> = (0..120u8).collect();
        let archive = dir.path().join("archive.tar.zst.age");
        std::fs::write(&archive, &payload).unwrap();

        let mid = MachineId::parse("0123456789abcdef").unwrap();
        let live_key = generation_key(
            cfg.prefix.as_deref(),
            &mid,
            "instrukt-home.20300106T030000Z.tar.zst.age",
        );
        let stg_key = staging_key(&live_key);

        storage
            .upload_to_staging(&archive, payload.len() as u64, &stg_key)
            .unwrap();
        storage
            .verify_staging(&stg_key, payload.len() as u64)
            .unwrap();
        storage
            .promote_staging(&stg_key, &live_key, payload.len() as u64)
            .unwrap();
        storage.write_validation_marker(&live_key).unwrap();
        storage.delete_object(&stg_key).unwrap();

        let st = mock.state.lock().unwrap();
        assert!(
            st.forbidden_header_seen.is_empty(),
            "forbidden streaming checksum headers sent: {:?}",
            st.forbidden_header_seen
        );
        assert_eq!(st.upload_part_count, 3);
        assert_eq!(
            st.upload_part_copy_ranges,
            vec!["bytes=0-49", "bytes=50-99", "bytes=100-119"]
        );
        assert_eq!(st.objects.get(&live_key).unwrap().0, payload);
        assert!(st.objects.contains_key(&validated_key(&live_key)));
        assert!(!st.objects.contains_key(&stg_key));
    }

    #[test]
    fn head_mismatch_and_promote_failure_delete_staging_object() {
        let mock = MockS3::start();
        let cfg = mock.config();
        let storage = S3Storage::new(&cfg);
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("small.tar.zst.age");
        std::fs::write(&archive, b"encrypted-bytes").unwrap();

        let live = "hosts/0123456789abcdef/instrukt-home.20300106T030000Z.tar.zst.age";
        let stg = staging_key(live);

        // Head mismatch deletes staging object.
        storage.upload_to_staging(&archive, 15, &stg).unwrap();
        mock.state.lock().unwrap().head_size_override = Some(999);
        assert!(storage.verify_staging(&stg, 15).is_err());
        assert!(!mock.state.lock().unwrap().objects.contains_key(&stg));

        // Promote error deletes staging object.
        mock.state.lock().unwrap().head_size_override = None;
        mock.state.lock().unwrap().fail_copy = true;
        storage.upload_to_staging(&archive, 15, &stg).unwrap();
        assert!(storage.promote_staging(&stg, live, 15).is_err());
        assert!(!mock.state.lock().unwrap().objects.contains_key(&stg));
    }

    #[test]
    fn prune_evicts_unvalidated_before_validated_and_cleans_only_own_orphans() {
        let mock = MockS3::start();
        let cfg = mock.config();
        let storage = S3Storage::new(&cfg);
        let own = MachineId::parse("0123456789abcdef").unwrap();
        let other = MachineId::parse("fedcba9876543210").unwrap();

        let g1_val = generation_key(
            cfg.prefix.as_deref(),
            &own,
            "instrukt-home.20300101T030000Z.tar.zst.age",
        );
        let g2_val = generation_key(
            cfg.prefix.as_deref(),
            &own,
            "instrukt-home.20300102T030000Z.tar.zst.age",
        );
        let g3_unval = generation_key(
            cfg.prefix.as_deref(),
            &own,
            "instrukt-home.20300103T030000Z.tar.zst.age",
        );
        let g4_val = generation_key(
            cfg.prefix.as_deref(),
            &own,
            "instrukt-home.20300104T030000Z.tar.zst.age",
        );
        let other_gen = generation_key(
            cfg.prefix.as_deref(),
            &other,
            "instrukt-home.20300101T030000Z.tar.zst.age",
        );
        let own_old_staging = staging_key(&generation_key(
            cfg.prefix.as_deref(),
            &own,
            "instrukt-home.20300101T030000Z.tar.zst.age",
        ));
        let own_fresh_staging = staging_key(&generation_key(
            cfg.prefix.as_deref(),
            &own,
            "instrukt-home.20300106T020000Z.tar.zst.age",
        ));
        let other_old_staging = staging_key(&other_gen);

        {
            let mut st = mock.state.lock().unwrap();
            for key in [&g1_val, &g2_val, &g3_unval, &g4_val, &other_gen] {
                st.objects.insert(
                    key.clone(),
                    (b"data".to_vec(), "2030-01-05T00:00:00Z".into()),
                );
            }
            for key in [&g1_val, &g2_val, &g4_val, &other_gen] {
                st.objects.insert(
                    validated_key(key),
                    (Vec::new(), "2030-01-05T00:00:00Z".into()),
                );
            }
            st.objects.insert(
                own_old_staging.clone(),
                (b"stale".to_vec(), "2030-01-04T00:00:00Z".into()),
            );
            st.objects.insert(
                own_fresh_staging.clone(),
                (b"fresh".to_vec(), "2030-01-06T02:00:00Z".into()),
            );
            st.objects.insert(
                other_old_staging.clone(),
                (b"other".to_vec(), "2030-01-04T00:00:00Z".into()),
            );
            st.uploads.insert(
                "u-old-own-stg".into(),
                MockUpload {
                    key: own_old_staging.clone(),
                    parts: BTreeMap::new(),
                    initiated: "2030-01-04T00:00:00Z".into(),
                },
            );
            st.uploads.insert(
                "u-old-own-live".into(),
                MockUpload {
                    key: g2_val.clone(),
                    parts: BTreeMap::new(),
                    initiated: "2030-01-04T00:00:00Z".into(),
                },
            );
            st.uploads.insert(
                "u-old-other".into(),
                MockUpload {
                    key: other_old_staging.clone(),
                    parts: BTreeMap::new(),
                    initiated: "2030-01-04T00:00:00Z".into(),
                },
            );
        }

        let now: Timestamp = "2030-01-06T03:00:00Z".parse().unwrap();
        let report = storage
            .prune_and_clean_orphans(&own, NonZeroU32::new(2).unwrap(), now)
            .unwrap();

        // Unvalidated g3 evicted first despite newer timestamp, then oldest validated g1.
        assert_eq!(
            report.pruned_generations,
            vec![g3_unval.clone(), g1_val.clone()]
        );
        assert_eq!(report.deleted_orphans, vec![own_old_staging.clone()]);
        assert_eq!(report.aborted_uploads.len(), 2);

        let st = mock.state.lock().unwrap();
        assert!(!st.objects.contains_key(&g3_unval));
        assert!(!st.objects.contains_key(&g1_val));
        assert!(!st.objects.contains_key(&validated_key(&g1_val)));
        assert!(st.objects.contains_key(&g2_val));
        assert!(st.objects.contains_key(&g4_val));
        assert!(st.objects.contains_key(&own_fresh_staging));
        assert!(st.objects.contains_key(&other_gen));
        assert!(st.objects.contains_key(&other_old_staging));
        assert!(st.uploads.contains_key("u-old-other"));
    }
}
