# Direct Fallback Eligibility - Delta Spec

## ADDED Requirements

### Requirement: Undispatched direct attempts may fall back

An outbound operation whose direct attempt failed before dispatch must be
eligible for the opportunistic packet fallback, and a refused fallback must
report the direct error before the refusal.

#### Scenario: No path to the peer
Given a direct send to a peer with no path
When the direct attempt fails before any link dispatch
Then the operation moves to the packet fallback
And if the fallback is refused the failure names the direct error first
