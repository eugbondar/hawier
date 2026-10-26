# Concepts at a glance

> Seven nouns. Everything else in this console is made of them.

## The seven nouns {#the-seven-nouns}

Read these in order: each one is built from the one before it.

Target :: A device. It has an identity (its controller id), it polls this server on a schedule, and it does what it is told. Everything else exists to give targets something to install.
Software module :: One installable thing with a name and a version — an operating system image, an application, a firmware blob. A module holds the files.
Artifact :: A file inside a software module, with its size and its hashes. This is what a device actually downloads.
Distribution set :: A named, versioned bundle of software modules — what you assign to a device. Devices are never given a module directly; they are given a set.
Action :: One job for one device: install this distribution set. It is created when you assign, and it lives until the device reports that it finished or failed.
Rollout :: A plan for giving the same distribution set to many devices, in waves, with rules about when to continue and when to stop.
Target filter :: A saved question about devices, written as a query. It picks who a rollout aims at, and it can hand out a distribution set automatically.

## How an update actually happens {#how-an-update-happens}

Nothing is ever pushed. The device asks, and the server answers.

1. The device polls the server on the schedule the tenant configured.
2. If it has a job waiting, the answer contains a link to that job.
3. The device follows the link, reads which modules and artifacts it needs, and downloads them.
4. It reports back as it goes: downloading, downloaded, and finally finished or failed.
5. The server records each report and moves the action, the device's status and any rollout that owns it.

The device side of that conversation is described in [what devices actually do](/help/reference/ddi#the-device-side). This is why nothing happens the instant you press a button: the device finds out at its next poll. See [why a device is not picking up its action](/help/start/troubleshooting#not-picking-up).

## Labels on a device {#labels}

There are five different ways to say something about a device and they do not mean the same thing. Getting them confused is the most common source of confusion in this product — see [naming and labelling devices](/help/inventory/identity#five-ways-to-label-a-device).

## What you never touch {#what-you-never-touch}

Actions are created for you — by an assignment, by a rollout, or by an auto-assignment rule. You can cancel one, but you cannot write one by hand. If you want a device to install something, you assign it a distribution set or you aim a rollout at it.
