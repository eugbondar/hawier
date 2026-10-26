# Spec Delta

## MODIFIED Requirements

### Requirement: Software module artifact transfer
`POST /rest/v1/softwaremodules/{id}/artifacts` SHALL accept a multipart upload. When the optional `sha1sum` or `md5sum` is given and does not match the content, the upload SHALL get 400.

The artifact file name (the `filename` query parameter, otherwise the multipart file name) SHALL be refused with 400, and nothing SHALL be stored, when it is empty or only spaces, longer than 256 bytes, contains a character outside printable ASCII (0x20–0x7E), or contains `/` or `\`. The error body is hawkBit's `hawkbit.server.error.repo.invalidParameter`. hawkBit accepts any name; this restriction is a deliberate divergence.

`GET .../artifacts/{aid}/download` SHALL support a single Range with 206. It SHALL send an RFC 6266 `Content-Disposition` with both `filename` and `filename*`, so names that predate the upload rule (for example in a database migrated from hawkBit) still download.

#### Scenario: Wrong checksum
- **WHEN** an artifact is uploaded with `md5sum=deadbeef` that does not match
- **THEN** the server answers 400 and stores nothing

#### Scenario: Non-ASCII filename
- **WHEN** an artifact is uploaded with a file name containing a non-ASCII character
- **THEN** the server answers 400 with `hawkbit.server.error.repo.invalidParameter` and the module's artifact list is unchanged

#### Scenario: Slash in filename refused
- **WHEN** an artifact is uploaded with the file name `a/b.bin`
- **THEN** the server answers 400 and stores nothing
