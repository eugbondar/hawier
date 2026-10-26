# Maintenance windows

> A repeating period during which a device is permitted to install.

## Maintenance windows {#maintenance-windows}

A window is attached to an assignment, not to a device. It is three things together: a repeating schedule, a duration, and a fixed time-zone offset.

Outside the window the device may **download** freely but is told not to install. Inside it, the job becomes an ordinary one.

## All three or none {#all-three}

The schedule, the duration and the offset are set together. Supplying some and not others is refused, and the schedule must have at least one window still ahead of it — a rule that expired is rejected rather than silently never firing.

## The schedule {#schedule}

A repeating expression: the field pattern that says "at 02:00 every Sunday". The duration says how long the window stays open once it starts.

The offset is fixed — a plain `+02:00`, not a named zone. Somewhere observing daylight saving will shift by an hour twice a year, which is the trade for a rule that means the same thing on every node.

## What the device sees {#what-the-device-sees}

The job description carries the window's state: **available** while it is open, **unavailable** while it is not, with the install step suppressed.

As a window approaches, the server shortens the sleep it hands the device so it polls a few times before the window opens rather than sleeping through it. The number of polls is a tenant setting, and there is a floor of thirty seconds so a fleet cannot be told to hammer the server.
