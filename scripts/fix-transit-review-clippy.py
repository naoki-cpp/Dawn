#!/usr/bin/env python3
"""Apply the two Transit review fixes requested by clippy."""

from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    content = path.read_text()
    if content.count(old) != 1:
        raise RuntimeError(f"expected one match in {path}, found {content.count(old)}")
    path.write_text(content.replace(old, new, 1))


replace_once(
    Path("crates/dawn-sector/src/node/transit.rs"),
    "    fn propose_transit_with_route(\n",
    "    #[cfg(test)]\n    fn propose_transit_with_route(\n",
)

replace_once(
    Path("crates/dawn-sector/src/transit/handoff.rs"),
    """            DomainEvent::SectorTransitAborted(event) if event.from == self.sector_id => {
                // SectorTransitAborted predates request_tick in its payload, so
                // route identity is the strongest safe match available. Never
                // let an old A -> B abort clear a newer A -> C request.
                if self
                    .pending_outgoing
                    .get(&event.ship_id)
                    .is_some_and(|pending| {
                        pending.identity.from == event.from && pending.identity.to == event.to
                    })
                {
                    self.pending_outgoing.remove(&event.ship_id);
                }
            }
""",
    """            DomainEvent::SectorTransitAborted(event)
                if event.from == self.sector_id
                    && self
                        .pending_outgoing
                        .get(&event.ship_id)
                        .is_some_and(|pending| {
                            pending.identity.from == event.from && pending.identity.to == event.to
                        }) =>
            {
                // SectorTransitAborted predates request_tick in its payload, so
                // route identity is the strongest safe match available. Never
                // let an old A -> B abort clear a newer A -> C request.
                self.pending_outgoing.remove(&event.ship_id);
            }
""",
)
