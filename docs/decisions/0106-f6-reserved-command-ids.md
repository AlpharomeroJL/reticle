# 0106, F6: reserved command ids live in a ledger disjoint from the registry

## Context

The v8.2 campaign adds many user-reachable commands across five phases (imports, 3D
exports, gallery, review, snapshots, agent, PCell, trace, waveform, classroom, plugins,
underlay, embed). The plan reserves every new command id NOW so no phase invents an id and
every planned action has one agreed name, menu location, and (later) chord (F6).

The v8.1 registry (`commands.rs`) is a static `&[CommandSpec]`, and every entry is a fully
working command: it carries a real `run: RunAs` target (a `Command` or an `AppOp`) and is
what the menu-parity test (`menu.rs::every_leaf_resolves_to_a_registry_id`), the keymap
conflict test, and the context-menu test read. There is no not-yet-functional registry
entry. So a reserved id for a feature that does not exist yet cannot be a `CommandSpec`: it
has no `run` target, and adding a placeholder one would put a dead command in the menus and
risk the parity and keymap invariants.

## Decision

F6 reservations are a separate ledger, not registry entries. `commands.rs` gains a
`RESERVED_CAMPAIGN_IDS: &[ReservedId]` table, where `ReservedId` is `(id, label, owner lane,
menu_path, chord)`. These carry no `run` target and never render, so the parity, keymap, and
context-menu tests are completely untouched. Chords are reserved as `None` and assigned by
the owning lane at implementation, so a reserved id can never introduce a keymap conflict.

The invariant is enforced by
`reserved_campaign_ids_are_well_formed_unique_and_disjoint_from_the_registry`: every reserved
id is dotted lowercase, has a label and an owner lane, is unique, and `spec(id)` returns
`None` (it is NOT a live command). When a lane ships a reserved command it moves the id into
`REGISTRY` with a real `run` and deletes the reserved row; the disjointness test then forces
the move to be complete (a reserved id cannot also be live). This is the "no phase invents
ids" mechanism.

There is no JSON fixture for F6 (unlike F1-F5): the contract is code (the reserved table and
its test), which is the byte-level artifact here.

## Consequences

Every campaign phase draws its command ids from a single reserved table agreed in Phase 0,
so ids, labels, and menu locations are decided once and cannot drift. The parity and keymap
tests that guard the live command subsystem never see a reserved id, so reserving 40-odd
future commands cost zero risk to the shipped menus (the Phase 0 gate re-ran the 20-test
parity/keymap baseline green with the reservations in place). A lane that ships a reserved
command must remove it from the ledger as it registers it, which the disjointness test makes
mechanical rather than a convention.
