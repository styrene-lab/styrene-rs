# Announce On Connect

## Intent

Keep a mobile node resolvable by the peers it talks to, and make an
unverified inbound message visible instead of silently ignored.

## Scope

Two daemon behaviours. The mobile node announces its delivery destination
whenever its transport becomes connected, including after a reconnect. When an
inbound message cannot be verified because the sender's identity is unknown,
the inbound worker logs that reason and requests the sender's path.

It excludes the People roster overhaul in the mobile shell, which is recorded
separately, and any change to the echo policy itself.

## Success criteria

- A mobile node announces once after its transport connects at start and once after every reconnect, and never while disconnected.
- An inbound message held for an unknown sender identity produces a diagnostic naming the message and sender and a path request for the sender.
- A phone that connects to the echo peer after the peer restarted receives an echo without any manual announce.
