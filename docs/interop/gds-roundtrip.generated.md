## Fixture `clean.gds`

Source (gdspy): dbu = 1.0 nm, cells = ['SUB', 'TOP'], element census = {'box': 2, 'polygon': 3, 'path': 0, 'text': 1, 'sref': 1, 'aref': 1}.

| tool | box | polygon | path | text | sref | aref |
|------|-----|---------|------|------|------|------|
| reticle | 2 | 3 | 0 | 1 | 1 | 1 |
| klayout | 2 | 3 | 0 | 1 | 1 | 1 |
| gdspy | 2 | 3 | 0 | 1 | 1 | 1 |

**reticle vs klayout:** geometry, labels, and instances all match after round-trip.

**gdspy vs klayout:** geometry, labels, and instances all match after round-trip.

**OASIS read test:** KLayout read Reticle's conformant-OASIS (`oasis_std`) output — PASS (2 cells, 6 shapes, dbu=0.001).

## Fixture `odd.gds`

Source (gdspy): dbu = 1.0 nm, cells = ['SUB', 'TOP'], element census = {'box': 4, 'polygon': 2, 'path': 0, 'text': 0, 'sref': 1, 'aref': 0}.

| tool | box | polygon | path | text | sref | aref |
|------|-----|---------|------|------|------|------|
| reticle | 4 | 2 | 0 | 0 | 1 | 0 |
| klayout | 4 | 2 | 0 | 0 | 1 | 0 |
| gdspy | 4 | 2 | 0 | 0 | 1 | 0 |

**reticle vs klayout:**
- cell `TOP`: instances differ (klayout=[['SUB', 45.0, False, 8000, 4000, 2.0, False]], reticle=[['SUB', 90.0, False, 8000, 4000, 2.0, False]])

**gdspy vs klayout:** geometry, labels, and instances all match after round-trip.

**OASIS read test:** KLayout read Reticle's conformant-OASIS (`oasis_std`) output — PASS (2 cells, 6 shapes, dbu=0.001).
