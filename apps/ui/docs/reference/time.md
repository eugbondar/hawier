# Time

> One format everywhere, in your zone, with the exact value one hover away.

## Reading a timestamp {#reading-a-timestamp}

Every time in this console is shown the same way: `HH:MM:SS dd.mm.yy`, in **your** browser's time zone.

Hovering shows the exact underlying value. On an object page the exact value is also printed in brackets next to it, because that is the value a query takes.

## Why the exact value is a number {#epoch}

Times travel between the server and everything else as a count of milliseconds. It has no zone, no format and no ambiguity — the formatting into something readable happens in your browser, which is why two people in different countries see the same instant written differently and correctly.

A raw number of milliseconds should never appear as a label in the interface. If one does, that is a bug.

## Times in queries {#in-queries}

Comparisons on a time field take the millisecond value, not the readable form:

```
lastControllerRequestAt=lt=1757000000000
createdAt=ge=1756000000000
```

Copy the number out of the brackets on an object page, or from the raw switch.

## Schedules {#schedules}

[Maintenance windows](/help/deployment/maintenance-windows#maintenance-windows) and [retention rules](/help/admin/retention-rules#cron) use repeating schedule expressions instead of instants.

A maintenance window's zone is a fixed offset like `+02:00`, not a named zone. Somewhere observing daylight saving shifts by an hour twice a year.
