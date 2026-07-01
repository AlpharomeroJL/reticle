# Routing

`reticle-route` connects nets across the layout while respecting obstacles and
spacing.

## Grid and maze

The router builds a routing grid over the area of interest and runs a maze search
(Lee expansion, or A\* with a distance heuristic via `pathfinding`) per net. The
grid encodes obstacles derived from existing geometry and from the design-rule
spacing, so a route is legal by construction.

## Rip-up and reroute

Nets compete for tracks. When a net cannot be routed because earlier nets have
filled the channels, the router rips up offending routes and retries in a different
order, trading a longer search for a higher completion rate. Cross-layer vias let a
route change layers to escape congestion.

## Reporting

The router reports how many nets it completed, the total routed length, and where
congestion remains, so a designer knows what to relieve.
