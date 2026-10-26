# Artifacts

> The files. Everything else in the repository is a wrapper around these.

## Artifacts {#artifacts}

An artifact is one file belonging to a software module, with its size and its hashes. This is what a device downloads.

The list on a module shows the filename, the size and a shortened SHA-1. The full hash is behind the copy control, and it is the same hash the device verifies against after downloading.

## Uploading {#uploading}

Choose a file on the module's page and upload it. The server hashes the file while it stores it. If you supplied hashes with the upload and they do not match what was received, the upload is rejected and nothing is kept.

The file name must be printable ASCII, at most 256 bytes, and must not contain `/` or `\`. Any other name is refused before anything is stored; rename the file and upload it again.

The same file uploaded to two modules is stored once: storage is addressed by content, so two identical uploads share one stored blob.

## Hashes {#hashes}

Each artifact carries SHA-1, MD5 and SHA-256. The device is given them in its job description and is expected to check what it downloaded.

A mismatch is the device's business to report, not something this server can detect for it.

## Deleting {#deleting}

Deleting an artifact removes it from the module. The stored file itself is only removed once nothing else refers to the same content.

Deleting a file a device is currently downloading will fail that download.

## Where the files live {#storage}

Artifact storage is provided by an [extension](/help/admin/extensions#extensions), not by the server itself. If the node has no storage extension, uploads and downloads answer 503 rather than silently doing something else. There is no built-in fallback, deliberately.

## Downloads {#downloads}

Downloads are streamed, never buffered whole, so an artifact's size is bounded by your storage rather than by the server's memory. Resumable and conditional downloads are supported, which is what lets a device on a poor link finish a large file across several attempts.
