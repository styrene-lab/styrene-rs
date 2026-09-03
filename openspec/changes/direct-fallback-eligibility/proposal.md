# Direct Fallback Eligibility

## Intent

Let a direct delivery that failed before it dispatched fall back to an
opportunistic packet, and keep the real transport error in front of the
operator when no fallback is possible.

## Scope

The outbound operation's fallback gate and the two direct-then-fallback call
sites in the messaging service. Nothing else changes.

## Success criteria

- A direct attempt that never dispatched (no path, no link) is eligible for the packet fallback.
- When the fallback is refused, the failure names the direct error first and the refusal second.
- An attempt that already accepted a resource, or that was cancelled, stays ineligible.
