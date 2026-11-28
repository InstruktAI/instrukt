//! Loopback HTTP S3-compatible mock server for real-binary `TestEnv` home
//! backup and restore integration tests (`home-backup`).
//!
//! Strictly verifies SigV4 `Authorization` headers and forbids `aws-chunked` /
//! streaming-trailer checksum headers on every `PutObject` and `UploadPart`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

#[derive(Clone)]
pub struct StoredObject {
    pub bytes: Vec<u8>,
    pub last_modified: String,
}

#[derive(Clone)]
pub struct MultipartUpload {
    pub key: String,
    pub parts: BTreeMap<u32, Vec<u8>>,
    pub initiated: String,
}

#[derive(Default)]
pub struct FakeS3State {
    pub objects: BTreeMap<String, StoredObject>,
    pub uploads: HashMap<String, MultipartUpload>,
    pub next_upload_id: u64,
    pub missing_auth_requests: Vec<String>,
    pub forbidden_headers: Vec<String>,
    pub head_size_override: Option<usize>,
    pub fail_copy_once: bool,
    pub aborted_uploads: Vec<(String, String)>,
}

pub struct FakeS3 {
    endpoint: String,
    state: Arc<Mutex<FakeS3State>>,
    server: Arc<tiny_http::Server>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FakeS3 {
    pub fn start() -> anyhow::Result<Self> {
        let server = Arc::new(
            tiny_http::Server::http("127.0.0.1:0")
                .map_err(|err| anyhow::anyhow!("bind fake S3 server: {err}"))?,
        );
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| anyhow::anyhow!("fake S3 server bound without IP address"))?;
        let endpoint = format!("http://127.0.0.1:{}", addr.port());
        let state = Arc::new(Mutex::new(FakeS3State::default()));
        let srv = Arc::clone(&server);
        let st = Arc::clone(&state);
        let handle = thread::spawn(move || {
            for req in srv.incoming_requests() {
                handle_request(req, &st);
            }
        });
        Ok(Self {
            endpoint,
            state,
            server,
            handle: Some(handle),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn state(&self) -> std::sync::MutexGuard<'_, FakeS3State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn put_object(&self, key: &str, bytes: Vec<u8>, last_modified: &str) {
        self.state().objects.insert(
            key.to_owned(),
            StoredObject {
                bytes,
                last_modified: last_modified.to_owned(),
            },
        );
    }

    pub fn put_incomplete_upload(&self, upload_id: &str, key: &str, initiated: &str) {
        self.state().uploads.insert(
            upload_id.to_owned(),
            MultipartUpload {
                key: key.to_owned(),
                parts: BTreeMap::new(),
                initiated: initiated.to_owned(),
            },
        );
    }

    pub fn keys(&self) -> Vec<String> {
        self.state().objects.keys().cloned().collect()
    }

    pub fn get_bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.state().objects.get(key).map(|o| o.bytes.clone())
    }

    pub fn wire_protocol_violations(&self) -> Vec<String> {
        let st = self.state();
        let mut issues = Vec::new();
        for req in &st.missing_auth_requests {
            issues.push(format!("missing SigV4 Authorization on {req}"));
        }
        for hdr in &st.forbidden_headers {
            issues.push(format!("forbidden streaming checksum header: {hdr}"));
        }
        issues
    }
}

impl Drop for FakeS3 {
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
        Some((_, key)) => percent_decode(key),
        None => String::new(),
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(hex_str, 16)
        {
            out.push(val);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn parse_byte_range(range: &str) -> Option<(usize, usize)> {
    let (a, b) = range.strip_prefix("bytes=")?.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn handle_request(mut req: tiny_http::Request, state: &Arc<Mutex<FakeS3State>>) {
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

    let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
    let has_sigv4 = headers
        .get("authorization")
        .is_some_and(|v| v.starts_with("AWS4-HMAC-SHA256 "));
    if !has_sigv4 {
        st.missing_auth_requests.push(format!("{method} {url}"));
    }
    if method == "PUT" && !headers.contains_key("x-amz-copy-source") {
        if headers
            .get("content-encoding")
            .is_some_and(|v| v.contains("aws-chunked"))
        {
            st.forbidden_headers
                .push("content-encoding:aws-chunked".into());
        }
        if headers
            .get("x-amz-content-sha256")
            .is_some_and(|v| v == "STREAMING-UNSIGNED-PAYLOAD-TRAILER")
        {
            st.forbidden_headers
                .push("x-amz-content-sha256:STREAMING-UNSIGNED-PAYLOAD-TRAILER".into());
        }
        for name in ["x-amz-trailer", "x-amz-sdk-checksum-algorithm"] {
            if headers.contains_key(name) {
                st.forbidden_headers.push(name.into());
            }
        }
    }

    match method.as_str() {
        "GET" if query.contains_key("uploads") => {
            let prefix = query.get("prefix").cloned().unwrap_or_default();
            let mut xml = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListMultipartUploadsResult><IsTruncated>false</IsTruncated>",
            );
            for (upload_id, upload) in &st.uploads {
                if upload.key.starts_with(&prefix) {
                    xml.push_str(&format!(
                        "<Upload><Key>{}</Key><UploadId>{}</UploadId><Initiated>{}</Initiated></Upload>",
                        xml_escape(&upload.key),
                        xml_escape(upload_id),
                        xml_escape(&upload.initiated),
                    ));
                }
            }
            xml.push_str("</ListMultipartUploadsResult>");
            let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
        }
        "GET" if query.get("list-type").map(String::as_str) == Some("2") => {
            let prefix = query.get("prefix").cloned().unwrap_or_default();
            let mut xml = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult><IsTruncated>false</IsTruncated>",
            );
            for (okey, obj) in &st.objects {
                if okey.starts_with(&prefix) {
                    xml.push_str(&format!(
                        "<Contents><Key>{}</Key><Size>{}</Size><LastModified>{}</LastModified></Contents>",
                        xml_escape(okey),
                        obj.bytes.len(),
                        xml_escape(&obj.last_modified),
                    ));
                }
            }
            xml.push_str("</ListBucketResult>");
            let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
        }
        "GET" => {
            if let Some(obj) = st.objects.get(&key).cloned() {
                let _ =
                    req.respond(tiny_http::Response::from_data(obj.bytes).with_status_code(200));
            } else {
                let _ = req.respond(tiny_http::Response::empty(404));
            }
        }
        "HEAD" => {
            if let Some(obj) = st.objects.get(&key) {
                let len = st.head_size_override.unwrap_or(obj.bytes.len());
                let mut resp = tiny_http::Response::empty(200);
                if let Ok(header) = tiny_http::Header::from_bytes("Content-Length", len.to_string())
                {
                    resp.add_header(header);
                }
                let _ = req.respond(resp);
            } else {
                let _ = req.respond(tiny_http::Response::empty(404));
            }
        }
        "POST" if query.contains_key("uploads") => {
            st.next_upload_id += 1;
            let upload_id = format!("upload-{}", st.next_upload_id);
            st.uploads.insert(
                upload_id.clone(),
                MultipartUpload {
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
                st.objects.insert(
                    upload.key,
                    StoredObject {
                        bytes: assembled,
                        last_modified: upload.initiated,
                    },
                );
                let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUploadResult><ETag>\"done\"</ETag></CompleteMultipartUploadResult>";
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
            } else {
                let _ = req.respond(tiny_http::Response::empty(404));
            }
        }
        "PUT" if headers.contains_key("x-amz-copy-source") && query.contains_key("partNumber") => {
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
            let Some((start, end)) = parse_byte_range(&range) else {
                let _ = req.respond(tiny_http::Response::empty(400));
                return;
            };
            let Some(slice) = st
                .objects
                .get(&src_key)
                .and_then(|o| o.bytes.get(start..=end))
                .map(ToOwned::to_owned)
            else {
                let _ = req.respond(tiny_http::Response::empty(404));
                return;
            };
            if let Some(upload) = st.uploads.get_mut(&upload_id) {
                upload.parts.insert(part_num, slice);
                let xml = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyPartResult><ETag>\"cp-{part_num}\"</ETag></CopyPartResult>"
                );
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
            } else {
                let _ = req.respond(tiny_http::Response::empty(404));
            }
        }
        "PUT" if headers.contains_key("x-amz-copy-source") => {
            if st.fail_copy_once {
                st.fail_copy_once = false;
                let _ = req.respond(tiny_http::Response::empty(403));
                return;
            }
            let copy_src = headers
                .get("x-amz-copy-source")
                .cloned()
                .unwrap_or_default();
            let src_key = strip_bucket(&copy_src);
            if let Some(obj) = st.objects.get(&src_key).cloned() {
                st.objects.insert(key, obj);
                let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CopyObjectResult><ETag>\"copied\"</ETag></CopyObjectResult>";
                let _ = req.respond(tiny_http::Response::from_string(xml).with_status_code(200));
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
            if let Some(upload) = st.uploads.get_mut(&upload_id) {
                upload.parts.insert(part_num, body);
                let mut resp = tiny_http::Response::empty(200);
                if let Ok(etag) = tiny_http::Header::from_bytes("ETag", format!("\"p-{part_num}\""))
                {
                    resp.add_header(etag);
                }
                let _ = req.respond(resp);
            } else {
                let _ = req.respond(tiny_http::Response::empty(404));
            }
        }
        "PUT" => {
            st.objects.insert(
                key,
                StoredObject {
                    bytes: body,
                    last_modified: "2030-01-06T03:00:00Z".into(),
                },
            );
            let mut resp = tiny_http::Response::empty(200);
            if let Ok(etag) = tiny_http::Header::from_bytes("ETag", "\"single\"") {
                resp.add_header(etag);
            }
            let _ = req.respond(resp);
        }
        "DELETE" if query.contains_key("uploadId") => {
            let upload_id = query.get("uploadId").cloned().unwrap_or_default();
            st.uploads.remove(&upload_id);
            st.aborted_uploads.push((key, upload_id));
            let _ = req.respond(tiny_http::Response::empty(204));
        }
        "DELETE" => {
            st.objects.remove(&key);
            let _ = req.respond(tiny_http::Response::empty(204));
        }
        _ => {
            let _ = req.respond(tiny_http::Response::empty(400));
        }
    }
}
