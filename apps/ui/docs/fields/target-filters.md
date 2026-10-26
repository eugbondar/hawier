# Target filter fields

## query {#query}

The [query](/help/reference/rsql#the-query-language) that selects devices. Checked when saved, so a filter that would fail is refused at creation.

## autoAssignDistributionSet {#auto-assign-distribution-set}

The distribution set this filter hands out automatically, if any. Empty means the filter only names devices and does nothing to them. See [auto-assignment](/help/deployment/auto-assignment#auto-assignment).

## autoAssignDistributionSetName {#auto-assign-distribution-set-name}

The same set by name, as a link.

## autoAssignActionType {#auto-assign-action-type}

How hard the automatic assignment pushes — see [action types](/help/deployment/action-types#action-types).

## autoAssignWeight {#auto-assign-weight}

Priority of the automatic assignment against other work on the device.

## confirmationRequired {#confirmation-required}

Whether the automatic assignment waits to be confirmed. See [confirmation](/help/deployment/confirmation#confirmation-required).
