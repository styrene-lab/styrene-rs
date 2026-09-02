# rns-resource-hardening - Baseline

### Requirement: Resource retries track rounds and progress

Inbound resource requests must adapt their request window from completed or timed-out rounds and count retries only for timeout-driven work rather than arriving fragments.

#### Scenario: Large transfer continues while fragments arrive
Given an inbound resource has more fragments than the initial request window
When valid fragments arrive and drain successive request rounds
Then active progress does not consume the retry limit
And the next round begins only after the current round drains or times out

### Requirement: Resource continuation and outstanding requests are bounded

Inbound resource requests must track outstanding fragment hashes, avoid requesting an in-flight fragment twice, request hashmap continuation only when the active window needs it, and expire a lost continuation gate so transfer cannot hang.

#### Scenario: Outstanding fragment remains in flight
Given an inbound resource has requested a bounded set of missing fragments
When another valid fragment arrives before the outstanding request times out
Then the in-flight missing fragment is not requested a second time
And the next request remains within the available window

#### Scenario: Hashmap continuation is lost
Given the active request window reaches unmapped fragment hashes and requests a hashmap continuation
When the continuation does not arrive before its retry deadline
Then the continuation gate expires and a bounded request is sent again
And the transfer eventually completes or emits one terminal timeout failure

### Requirement: Resource admission and fragments are bounded before allocation

Resource advertisements must be checked against data-size, transfer-size, part-count, destination, effective-Link, and interface limits before receiver state or proportional part storage is allocated. Effective-Link MTU negotiation is owned by `reticulum-1-5-parity-wave`; this requirement consumes that value.

#### Scenario: Advertisement exceeds a resource cap
Given an advertisement exceeds one configured size or part-count cap by one
When the resource manager receives the advertisement
Then no receiver or part-tracking allocation is created
And no resource request packet is emitted

#### Scenario: Effective MTU constrains resource fragments
Given the verified Link MTU work supplies an effective MTU smaller than the default resource MTU
When a resource sender sizes its advertisement, hashmap updates, and fragments
Then every resulting packet fits the supplied effective MTU
And no second MTU negotiation or signaling policy is introduced

### Requirement: Split resources have one bounded terminal lifecycle

Split resources must prepare segments incrementally outside the global transport lock, preserve byte-exact metadata and assembly semantics, and release all original-resource state with exactly one terminal result on completion, segment cancellation, timeout, packet-build failure, segment-build failure, or assembly mismatch. Generic Link-close cancellation is owned by `reticulum-1-5-parity-wave`.

#### Scenario: Multi-segment resource completes
Given an outbound payload requires multiple resource segments and includes metadata
When every segment is requested and proved
Then only the first segment carries and strips the metadata block
And the receiver emits one verified completion containing byte-exact original data while both sides release all segment state

#### Scenario: Split resource is cancelled after partial assembly
Given one or more split-resource segments completed and a later segment is active
When either endpoint cancels the active segment
Then state keyed by both the segment hash and original resource hash is released
And the observer receives exactly one terminal cancellation failure with accumulated progress
