//! Artifact storage, host side.
//!
//! hawier does not know what a storage backend is. It asks the storage
//! extension *where* an artifact is and gets back a descriptor — a URL or a
//! path — and then does the IO itself. Filesystem, S3, anything else: the
//! difference lives in a separate process, and none of it is compiled in.
//!
//! The host keeps exactly the parts an extension must not hold: the byte
//! stream, the upload spool, and the rule that a descriptor is data rather
//! than authority (a path is only honoured inside the configured root).
//!
//! There is no fallback. With no storage extension configured, or with one
//! that will not answer, artifact requests are `503` with `Retry-After`, so
//! a fleet backs off instead of being told something untrue.

pub mod sha1_path;

use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use anyhow::Context;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use hawier_api::ext::{points, StorageLocate, StorageLocation, StorageOp};
use md5::Md5;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;

use crate::error::{AppError, AppResult};
use crate::ext::{self, wasm::Extension};

/// Streaming read buffer for downloads (hot path — sized for throughput).
const READ_BUF: usize = 256 * 1024;

/// Digests and size of a stored artifact.
#[derive(Debug, Clone)]
pub struct StoredArtifact {
    pub sha1: String,
    pub md5: String,
    pub sha256: String,
    pub size: i64,
}

/// How a download should be served, decided by one `locate`.
pub enum Download {
    /// Send the device here; the payload never enters this process.
    Redirect(String),
    /// Serve it ourselves from this stream.
    Reader(ArtifactReader),
    /// The store does not have it.
    Absent,
}

/// A readable artifact: its full size plus the byte stream.
pub struct ArtifactReader {
    pub total_size: u64,
    pub stream: std::pin::Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>,
}

/// Host-side storage: an extension for the decisions, this for the bytes.
#[derive(Clone)]
pub struct Storage {
    extensions: ext::ExtRegistry,
    /// Local scratch. Uploads are named by their own sha1, so they are
    /// written and hashed here before anything can be told where they go.
    spool: PathBuf,
    /// A `Path` descriptor is only honoured inside this root.
    root: PathBuf,
    client: reqwest::Client,
}

impl Storage {
    pub fn new(extensions: ext::ExtRegistry, artifact_dir: &Path, client: reqwest::Client) -> Self {
        Self {
            extensions,
            spool: artifact_dir.join(".tmp"),
            root: artifact_dir.to_path_buf(),
            client,
        }
    }

    async fn locate(&self, tenant: &str, sha1: &str, op: StorageOp) -> AppResult<StorageLocation> {
        sha1_path::validate(sha1)?;
        let extension: &Extension = self.extensions.storage()?;
        ext::call(
            extension,
            points::STORAGE_LOCATE,
            &StorageLocate {
                tenant: tenant.to_string(),
                sha1: sha1.to_string(),
                op,
            },
        )
        .await
    }

    /// A path from an extension is a suggestion, not a permission: resolve it
    /// and refuse anything outside the configured root, so a compromised or
    /// buggy extension cannot make the host read `/etc/shadow`.
    fn accept_path(&self, path: &str) -> AppResult<PathBuf> {
        let candidate = PathBuf::from(path);
        if !candidate.is_absolute() || candidate.components().any(|c| c.as_os_str() == "..") {
            return Err(AppError::Internal(anyhow::anyhow!(
                "storage extension returned a non-absolute or traversing path"
            )));
        }
        if !candidate.starts_with(&self.root) {
            return Err(AppError::Internal(anyhow::anyhow!(
                "storage extension returned a path outside the artifact root"
            )));
        }
        Ok(candidate)
    }

    pub async fn store(
        &self,
        tenant: &str,
        stream: impl Stream<Item = std::io::Result<Bytes>> + Send + Unpin,
    ) -> AppResult<StoredArtifact> {
        let (tmp_path, stored) = spool_and_hash(&self.spool, stream).await?;
        let outcome = self.place(tenant, &stored, &tmp_path).await;
        tokio::fs::remove_file(&tmp_path).await.ok();
        outcome?;
        Ok(stored)
    }

    async fn place(&self, tenant: &str, stored: &StoredArtifact, tmp_path: &Path) -> AppResult<()> {
        // Content-addressed: identical bytes are already the same object.
        if self.exists(tenant, &stored.sha1).await? {
            return Ok(());
        }
        match self.locate(tenant, &stored.sha1, StorageOp::Put).await? {
            StorageLocation::Url {
                url,
                method,
                headers,
                ..
            } => {
                let file = tokio::fs::File::open(tmp_path)
                    .await
                    .context("reopening the upload spool")?;
                let mut request = self
                    .client
                    .request(parse_method(&method)?, url)
                    .header(reqwest::header::CONTENT_LENGTH, stored.size)
                    .body(reqwest::Body::wrap_stream(ReaderStream::new(file)));
                for (name, value) in headers {
                    request = request.header(name, value);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|e| AppError::Unavailable(format!("artifact upload failed: {e}")))?;
                if !response.status().is_success() {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "artifact store answered {} to the upload",
                        response.status()
                    )));
                }
                Ok(())
            }
            StorageLocation::Path { path } => {
                let target = self.accept_path(&path)?;
                if let Some(parent) = target.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .context("creating the artifact directory")?;
                }
                tokio::fs::rename(tmp_path, &target)
                    .await
                    .context("moving the artifact into place")?;
                Ok(())
            }
            StorageLocation::Absent => Err(AppError::Internal(anyhow::anyhow!(
                "storage extension has nowhere to put an artifact"
            ))),
        }
    }

    pub async fn open(
        &self,
        tenant: &str,
        sha1: &str,
        range: Option<(u64, u64)>,
    ) -> AppResult<Option<ArtifactReader>> {
        let location = self.locate(tenant, sha1, StorageOp::Get).await?;
        self.read(location, range).await
    }

    /// Turn a resolved location into a byte stream.
    async fn read(
        &self,
        location: StorageLocation,
        range: Option<(u64, u64)>,
    ) -> AppResult<Option<ArtifactReader>> {
        match location {
            StorageLocation::Absent => Ok(None),
            StorageLocation::Path { path } => open_file(&self.accept_path(&path)?, range).await,
            StorageLocation::Url {
                url,
                method,
                headers,
                ..
            } => {
                let mut request = self.client.request(parse_method(&method)?, url);
                for (name, value) in headers {
                    request = request.header(name, value);
                }
                if let Some((start, end)) = range {
                    request =
                        request.header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
                }
                let response = request
                    .send()
                    .await
                    .map_err(|e| AppError::Unavailable(format!("artifact download failed: {e}")))?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                if !response.status().is_success() {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "artifact store answered {}",
                        response.status()
                    )));
                }
                let total_size = total_size_of(&response);
                let stream = response
                    .bytes_stream()
                    .map(|chunk| chunk.map_err(std::io::Error::other));
                Ok(Some(ArtifactReader {
                    total_size,
                    stream: Box::pin(stream),
                }))
            }
        }
    }

    pub async fn exists(&self, tenant: &str, sha1: &str) -> AppResult<bool> {
        match self.locate(tenant, sha1, StorageOp::Head).await? {
            StorageLocation::Absent => Ok(false),
            StorageLocation::Path { path } => {
                let path = self.accept_path(&path)?;
                Ok(tokio::fs::try_exists(&path).await.unwrap_or(false))
            }
            StorageLocation::Url { url, .. } => {
                let response = self
                    .client
                    .head(url)
                    .send()
                    .await
                    .map_err(|e| AppError::Unavailable(format!("artifact head failed: {e}")))?;
                Ok(response.status().is_success())
            }
        }
    }

    pub async fn delete(&self, tenant: &str, sha1: &str) -> AppResult<()> {
        match self.locate(tenant, sha1, StorageOp::Delete).await? {
            StorageLocation::Absent => Ok(()),
            StorageLocation::Path { path } => {
                let path = self.accept_path(&path)?;
                match tokio::fs::remove_file(&path).await {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(anyhow::Error::from(e).into()),
                }
            }
            StorageLocation::Url { url, method, .. } => {
                self.client
                    .request(parse_method(&method)?, url)
                    .send()
                    .await
                    .map_err(|e| AppError::Unavailable(format!("artifact delete failed: {e}")))?;
                Ok(())
            }
        }
    }

    /// Resolve a download with a single `locate`.
    ///
    /// One call, not two: this is the artifact path, and asking the
    /// extension once per download rather than once to check for a redirect
    /// and again to open the bytes halves its cost on the busiest route hawier
    /// has.
    ///
    /// A URL the extension marked `redirect_ok` is handed to the device as a
    /// 302 — the point of an object store, since the payload then leaves
    /// hawier entirely while every decision about it (authentication, module
    /// authorisation, the sha1 ETag, the DOWNLOAD status) stays here.
    pub async fn for_download(
        &self,
        tenant: &str,
        sha1: &str,
        range: Option<(u64, u64)>,
    ) -> AppResult<Download> {
        match self.locate(tenant, sha1, StorageOp::Get).await? {
            StorageLocation::Absent => Ok(Download::Absent),
            StorageLocation::Url {
                url, redirect_ok, ..
            } if redirect_ok => Ok(Download::Redirect(url)),
            location => Ok(self
                .read(location, range)
                .await?
                .map(Download::Reader)
                .unwrap_or(Download::Absent)),
        }
    }
}

fn parse_method(method: &str) -> AppResult<reqwest::Method> {
    reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| {
        AppError::Internal(anyhow::anyhow!(
            "storage extension asked for an unusable method '{method}'"
        ))
    })
}

/// Full object size, which for a partial response lives in the total of
/// `Content-Range: bytes start-end/total`, not in `Content-Length`.
fn total_size_of(response: &reqwest::Response) -> u64 {
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        if let Some(total) = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next().map(str::to_string))
            .and_then(|total| total.trim().parse::<u64>().ok())
        {
            return total;
        }
    }
    response.content_length().unwrap_or(0)
}

async fn open_file(path: &Path, range: Option<(u64, u64)>) -> AppResult<Option<ArtifactReader>> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::Error::from(e).into()),
    };
    let total_size = file.metadata().await.context("artifact metadata")?.len();
    let stream: std::pin::Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>> = match range {
        Some((start, end)) => {
            file.seek(SeekFrom::Start(start))
                .await
                .context("seeking the artifact")?;
            let len = end.saturating_sub(start) + 1;
            Box::pin(ReaderStream::with_capacity(file.take(len), READ_BUF))
        }
        None => Box::pin(ReaderStream::with_capacity(file, READ_BUF)),
    };
    Ok(Some(ArtifactReader { total_size, stream }))
}

/// Spool an upload to a temporary file, computing sha1/md5/sha256 in one
/// pass, and hand back the path plus the digests.
///
/// This stays in the host because it is the byte path: content addressing
/// means the object's name *is* its sha1, so nothing can be placed until the
/// last byte has been read, and no extension may see those bytes. Never
/// buffers the artifact in memory. The caller owns the returned path.
pub(crate) async fn spool_and_hash(
    tmp_dir: &Path,
    mut stream: impl Stream<Item = std::io::Result<Bytes>> + Send + Unpin,
) -> AppResult<(PathBuf, StoredArtifact)> {
    tokio::fs::create_dir_all(tmp_dir)
        .await
        .context("creating artifact tmp dir")?;
    let tmp_path = tmp_dir.join(format!("upload-{}", crate::db::model::generate_sec_token()));

    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .context("creating temp artifact file")?;
    let mut sha1 = Sha1::new();
    let mut md5 = Md5::new();
    let mut sha256 = Sha256::new();
    let mut size: i64 = 0;

    let result: AppResult<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AppError::BadRequest(format!("upload read: {e}")))?;
            sha1.update(&chunk);
            md5.update(&chunk);
            sha256.update(&chunk);
            size += chunk.len() as i64;
            file.write_all(&chunk).await.context("writing artifact")?;
        }
        file.flush().await.context("flushing artifact")?;
        Ok(())
    }
    .await;

    drop(file);
    if let Err(e) = result {
        tokio::fs::remove_file(&tmp_path).await.ok();
        return Err(e);
    }

    Ok((
        tmp_path,
        StoredArtifact {
            sha1: hex::encode(sha1.finalize()),
            md5: hex::encode(md5.finalize()),
            sha256: hex::encode(sha256.finalize()),
            size,
        },
    ))
}

pub fn ensure_base_dir(base: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(base)
        .with_context(|| format!("creating artifact dir {}", base.display()))
}
