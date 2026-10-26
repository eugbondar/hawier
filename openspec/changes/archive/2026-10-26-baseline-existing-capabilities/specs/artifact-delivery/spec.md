# Spec Delta

## Purpose

How devices obtain artifact bytes through DDI: authorization by deployment,
streaming, resumable and conditional transfer, hash companions, redirects to an
external store, and the declared refusal when no storage is configured.

## ADDED Requirements

### Requirement: Download is authorized by deployment
A device SHALL be able to list or download the artifacts of a software module only if the target has some action, active or closed, whose distribution set contains that module. Otherwise the server SHALL answer 404.

#### Scenario: Stranger target
- **WHEN** a target that was never assigned a set containing module M requests M's artifact
- **THEN** the server answers 404

### Requirement: Artifact listing
`GET softwaremodules/{smId}/artifacts` SHALL return a JSON array of the module's artifacts. Each entry SHALL carry the same fields and `_links` (`download-http`, `md5sum-http`) as in the deployment base.

#### Scenario: Listing matches deployment chunk
- **WHEN** a device lists the artifacts of a module it was assigned
- **THEN** every artifact has the same filename, hashes, size and links as in its deploymentBase chunk

### Requirement: Streaming full download
A full download SHALL answer 200 with these headers:
- `ETag: "<sha1>"`
- `Accept-Ranges: bytes`
- `Content-Type: application/octet-stream`
- `Content-Disposition: attachment;filename=<name>`
- `Content-Length` equal to the recorded artifact size

The body SHALL be streamed. The server SHALL NOT buffer the whole file in memory.

#### Scenario: Full download headers
- **WHEN** an authorized device GETs an artifact without Range or conditional headers
- **THEN** it receives 200, the ETag equals the quoted sha1, and the bytes hash to that sha1

### Requirement: Conditional and partial transfer
- An `If-Match` other than `*` that does not equal the artifact sha1 (quotes ignored) SHALL get 412 with an empty body.
- An `If-None-Match` equal to the sha1 SHALL get 304 with the ETag.
- A single range `bytes=a-b`, `bytes=a-` or `bytes=-n` SHALL get 206 with `Content-Range: bytes a-b/total` and exactly that slice. The end SHALL be clamped to the last byte.
- A multi-range request, or a Range without the `bytes=` unit, SHALL be served as a full 200.

#### Scenario: Resume a slice
- **WHEN** a device requests `Range: bytes=4-7`
- **THEN** it receives 206, `Content-Range: bytes 4-7/<size>`, and 4 bytes

#### Scenario: Stale validator
- **WHEN** a device sends `If-Match` with a sha1 that is not this artifact's
- **THEN** it receives 412

#### Scenario: Already have it
- **WHEN** a device sends `If-None-Match` with the artifact's sha1
- **THEN** it receives 304

### Requirement: MD5SUM companion
`GET .../artifacts/{fileName}.MD5SUM` SHALL answer 200 `text/plain` with the body `<md5>  <fileName>\n` and `Content-Disposition: attachment;filename=<fileName>.MD5SUM`. This SHALL come from recorded metadata and SHALL NOT need storage.

#### Scenario: MD5 file
- **WHEN** a device follows `_links.md5sum-http`
- **THEN** it receives the md5 and the filename separated by two spaces

### Requirement: Download is recorded as progress
A full (non-Range) download SHALL write a DOWNLOAD status `Target downloads <file>` against the target's current active action. Range requests SHALL NOT write a status.

#### Scenario: Download appears in history
- **WHEN** a device downloads its artifact in full during an update
- **THEN** the action history contains a DOWNLOAD entry between RUNNING and DOWNLOADED

### Requirement: Storage is located by an extension, never built in
The server SHALL obtain the location of artifact bytes only from the storage extension, as one of three answers: a URL, a path, or absent. With no storage extension configured, download and upload SHALL answer 503 with `Retry-After`. The node SHALL still start and serve polling and deployments. There SHALL be no built-in fallback storage.

#### Scenario: No storage module
- **WHEN** a node runs without `HAWIER_EXT_STORAGE` and a device requests an artifact
- **THEN** it receives 503 with a Retry-After header, and polling still answers 200

#### Scenario: Storage module present
- **WHEN** the filesystem storage module is configured and the artifact exists
- **THEN** the download succeeds with the correct bytes

### Requirement: Redirect to an external store
When storage answers with a URL that permits redirect, the download SHALL answer 302 with:
- `Location` set to that URL
- `ETag: "<sha1>"`
- `Accept-Ranges: bytes`

A full download SHALL still be recorded as DOWNLOAD. When storage answers with a URL that does not permit redirect, the server SHALL proxy the bytes:
- a Range SHALL be forwarded
- a store 404 SHALL become 404
- a store transport failure SHALL become 503 with Retry-After

#### Scenario: Presigned S3 URL
- **WHEN** the S3 storage module answers GET with a presigned URL
- **THEN** the device receives 302 to that URL and the bytes do not pass through the node

### Requirement: Path descriptors stay inside the artifact root
A path answer SHALL be accepted only if it is absolute, has no `..` segment, and lies under `HAWIER_ARTIFACT_DIR`. Otherwise the server SHALL refuse it without performing any IO. A sha1 that is not hex, or is shorter than four characters, SHALL be rejected before storage is asked. A path whose file does not exist SHALL answer 404.

#### Scenario: Traversal refused
- **WHEN** a storage module answers a path outside the artifact root
- **THEN** the server performs no file IO and answers an error

### Requirement: hawkBit-compatible on-disk layout
The filesystem layout of artifact content SHALL be `{root}/{TENANT}/{sha1[-4..-2]}/{sha1[-2..]}/{sha1}`, with the tenant upper-cased. This is hawkBit's file storage layout. Content SHALL be addressed by sha1 and scoped to the tenant.

#### Scenario: Fan-out on the last four hex digits
- **WHEN** an artifact with sha1 ending `46ed` is stored for tenant `default`
- **THEN** its file is at `<root>/DEFAULT/46/ed/<sha1>`

### Requirement: Upload hashing and content deduplication
An artifact upload SHALL be spooled and hashed (sha1, md5, sha256) in one pass. Content already present in storage SHALL NOT be written again. Content SHALL be deleted from storage only when no artifact record references its sha1 any longer. The upload body limit SHALL be `HAWIER_MAX_ARTIFACT_SIZE`, default 1 GiB.

#### Scenario: Shared content survives
- **WHEN** two modules carry the same file and one module's artifact is deleted
- **THEN** the other module's artifact still downloads
