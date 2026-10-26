# What devices actually do

> The device's side of the conversation, described for the person watching it from here.

## The device side {#the-device-side}

Nothing is pushed. A device asks this server for work on a schedule, does what it is told, and reports back. Everything you see in the console is the record of that conversation.

Understanding it is what turns "nothing is happening" into "it will happen at 14:07".

## Polling {#polling}

The device calls in and is told two things: how long to sleep before calling again, and links to whatever is waiting for it. If nothing is waiting, it just sleeps.

The sleep comes from the tenant's [polling interval](/help/admin/settings#polling-time). It is shortened as a [maintenance window](/help/deployment/maintenance-windows#maintenance-windows) approaches so the device is awake when it opens.

The device follows the links it is given; it does not build addresses of its own. That is what lets the fleet be moved between nodes without touching a device.

## Authentication {#authentication}

Either a per-device [security token](/help/inventory/targets#security-token), or a shared gateway token for the whole tenant, depending on what the tenant [allows](/help/admin/settings#authentication-targettoken-enabled).

The shared token is what makes plug-and-play registration possible: a device presenting it with an identity nobody has seen is created on the spot, given a token of its own, and asked to report its attributes.

## Reporting attributes {#attributes}

The device sends what it knows about itself — hardware revision, firmware version, serials. It can merge into what is stored, replace it wholesale, or remove entries. See [attributes](/help/inventory/identity#attributes).

## Getting a job {#getting-a-job}

When something is waiting, the poll answer links to it. Fetching that link is what moves the action to [retrieved](/help/glossary#retrieved) — recorded once, so two devices sharing an identity cannot both claim it.

The job describes each module as a chunk: its type, its name, its version, and every file with its size, its hashes and where to get it. Module [metadata marked visible](/help/inventory/identity#visible-to-device) travels with it.

The job also says how hard to push — download and install each carry their own instruction, which is how [download only](/help/deployment/action-types#downloadonly) is expressed, and how a closed maintenance window suppresses the install step while still allowing the download.

## Reporting back {#feedback}

As it works, the device reports: started downloading, downloaded, still going, finished, failed. Each report becomes a row in the action's [status history](/help/deployment/actions#status-history), with whatever message the device attached.

A device reporting the same thing repeatedly does not fill the history — the row counts the repeats instead.

Success and failure both close the action. Reporting on an action that is already closed is refused, which is how a device that has been out of contact learns its job is gone.

## Cancelling {#cancelling}

A cancellation is offered to the device the same way a job is, and the device answers. It may accept, or it may [refuse](/help/glossary#cancel-rejected) — a device past the point of no return is allowed to say so, and it then finishes.

## Declaring what is installed {#installed-base}

A device can also state what it is running, outside of any action. This is how a device that was updated offline reports the result.

> That statement is taken at face value. Nothing verifies it, and the device is marked in sync with whatever set it named. See [sharp edges](/help/reference/sharp-edges#offline-declaration).

## Confirming {#confirmation}

With the [confirmation flow](/help/deployment/confirmation#the-confirmation-flow) on, the device is offered its job and must confirm before it may install. It can confirm, deny, or be exempted permanently by [auto-confirmation](/help/deployment/confirmation#auto-confirmation).

## Downloading {#downloading}

Files are streamed, and downloads can be resumed and validated, so a device on a poor link can finish a large file across several attempts. A checksum file sits beside each artifact.

Without a storage [extension](/help/admin/extensions#no-extension-no-fallback), downloads answer 503.
