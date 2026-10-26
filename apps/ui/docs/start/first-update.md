# Get one device updated

> The shortest path from a file on your disk to a device running it.

## Before you start {#before-you-start}

You need the device to already be polling this server. A device that has never polled will not appear in the target list unless someone created it by hand, and a target created by hand cannot be updated until the real device introduces itself.

Check the [targets](/targets) list for its controller id. A device that has just been plugged in shows as [registered](/help/glossary#registered).

## Step by step {#step-by-step}

1. **Create a software module.** [Software Modules](/software-modules) then **New**. Give it a name, a version and a type. This is the container; it has no files yet.
2. **Upload the artifact.** Open the module you just made and upload the file on its page. The server hashes it while it stores it, and the device will verify the same hash after downloading.
3. **Create a distribution set.** [Distribution Sets](/distribution-sets) then **New**. Name it, version it, pick a type. The type decides which kinds of module the set must contain before it counts as [complete](/help/repository/distribution-sets#complete).
4. **Attach the module** to the set on the set's page. Watch **complete** turn true; an incomplete set cannot be assigned.
5. **Assign it.** Open the device in [Targets](/targets), find **Assign distribution set**, pick the set, choose an [action type](/help/deployment/action-types#action-types), and assign.
6. **Wait for the poll.** The device's status becomes [pending](/help/glossary#pending) immediately, and stays there until the device asks for work. How long depends on the tenant's polling interval — see [polling](/help/admin/settings#polling-time).
7. **Watch the action.** The device page lists its actions; open the one you just created to see every report the device has sent, with timestamps.

## When it worked {#when-it-worked}

The action reaches [finished](/help/glossary#finished), the device's update status becomes [in_sync](/help/glossary#in-sync), and its **installed distribution set** is the one you assigned.

## When it did not {#when-it-did-not}

The action reaches [error](/help/glossary#error) and the device's status becomes error. The action's status history holds whatever the device said about why — the messages in it come from the device, not from this server.

See [troubleshooting](/help/start/troubleshooting#not-picking-up).
