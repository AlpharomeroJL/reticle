#!/usr/bin/env python3
"""GDS interop harness: fixtures, per-tool round-trips, and a divergence report.

Runs inside the pinned hpretl/iic-osic-tools container (KLayout 0.29.x + gdspy 1.6),
headless. Reticle's own round-trip is produced on the host by the reticle-roundtrip
Rust binary and dropped into the same work dir; this script normalizes every tool's
output with a single authoritative reader (KLayout's klayout.db) so any divergence is
attributable to the tool that WROTE the file, not to a reader difference.

Subcommands (workdir holds fixtures/*.gds and each tool's <fixture>.<tool>.gds):
  fixtures <workdir>            write clean.gds and odd.gds with gdspy
  roundtrip <tool> <in> <out>  read <in>, write <out> with tool in {klayout,gdspy}
  report <workdir> <out.md>    normalize every *.gds, diff the tools, write the report

Units: 1 user unit = 1 um, 1 dbu = 1 nm (precision 1e-9), matching Reticle's GDS.
"""

import sys
import json

UM = 1e-6
NM = 1e-9


# --------------------------------------------------------------------------- #
# Fixtures (gdspy)
# --------------------------------------------------------------------------- #
def make_fixtures(workdir):
    import gdspy

    def new_lib():
        # A fresh library per file; gdspy keeps global cell state otherwise.
        gdspy.current_library = gdspy.GdsLibrary()
        return gdspy.current_library

    # --- clean.gds: well-formed, rectilinear-plus-45, one level of hierarchy. ---
    lib = new_lib()
    sub = lib.new_cell("SUB")
    sub.add(gdspy.Rectangle((0, 0), (1.0, 1.0), layer=68, datatype=20))
    top = lib.new_cell("TOP")
    # A rectangle (BOUNDARY on layer 67/20).
    top.add(gdspy.Rectangle((0, 0), (2.0, 1.0), layer=67, datatype=20))
    # A Manhattan L-polygon.
    top.add(gdspy.Polygon([(3, 0), (5, 0), (5, 2), (4, 2), (4, 1), (3, 1)],
                          layer=68, datatype=20))
    # A 45-degree polygon (a diamond).
    top.add(gdspy.Polygon([(7, 1), (8, 0), (9, 1), (8, 2)], layer=69, datatype=20))
    # A path with flush ends.
    top.add(gdspy.FlexPath([(0, 4), (3, 4), (3, 6)], width=0.3, ends="flush",
                          layer=70, datatype=20))
    # A text label.
    top.add(gdspy.Label("PIN", (1.0, 0.5), layer=67, texttype=5))
    # A cell reference and a 2x2 array of the sub-cell.
    top.add(gdspy.CellReference(sub, (6, 4)))
    top.add(gdspy.CellArray(sub, 2, 2, (2.0, 2.0), (10, 0)))
    lib.write_gds(f"{workdir}/clean.gds")

    # --- odd.gds: seeded quirks that tend to surface writer/reader divergences. ---
    lib = new_lib()
    sub = lib.new_cell("SUB")
    sub.add(gdspy.Rectangle((0, 0), (0.5, 0.5), layer=68, datatype=20))
    top = lib.new_cell("TOP")
    # A 1 dbu (1 nm) thin sliver rectangle: precision/round-trip stress.
    top.add(gdspy.Rectangle((0, 0), (0.001, 1.0), layer=67, datatype=20))
    # A path with ROUND ends: writers differ on how (or whether) they preserve the
    # round cap, so KLayout's rendered polygon of each writer's output can diverge.
    top.add(gdspy.FlexPath([(2, 0), (4, 0)], width=0.4, ends="round",
                          layer=70, datatype=20))
    # A path with custom (half-width) extensions (GDS pathtype 4 semantics).
    top.add(gdspy.FlexPath([(2, 3), (4, 3)], width=0.4, ends=(0.2, 0.2),
                          layer=70, datatype=20))
    # A polygon with a duplicate consecutive vertex (degenerate edge).
    top.add(gdspy.Polygon([(6, 0), (6, 0), (7, 0), (7, 1), (6, 1)],
                          layer=68, datatype=20))
    # A reference rotated 45 degrees with magnification 2: transform fidelity.
    top.add(gdspy.CellReference(sub, (8, 4), rotation=45, magnification=2.0))
    # A negative-coordinate rectangle.
    top.add(gdspy.Rectangle((-2.0, -1.0), (-1.0, 0.0), layer=69, datatype=20))
    lib.write_gds(f"{workdir}/odd.gds")
    print("wrote clean.gds and odd.gds", file=sys.stderr)


# --------------------------------------------------------------------------- #
# Per-tool round-trip (read a GDS, write it back)
# --------------------------------------------------------------------------- #
def roundtrip(tool, src, dst):
    if tool == "klayout":
        import klayout.db as db
        ly = db.Layout()
        ly.read(src)
        ly.write(dst)
    elif tool == "gdspy":
        import gdspy
        gdspy.current_library = gdspy.GdsLibrary()
        lib = gdspy.GdsLibrary()
        lib.read_gds(src)
        lib.write_gds(dst)
    else:
        raise SystemExit(f"unknown tool {tool!r}")
    print(f"{tool}: {src} -> {dst}", file=sys.stderr)


# --------------------------------------------------------------------------- #
# Normalization (single authoritative reader: klayout.db)
# --------------------------------------------------------------------------- #
def normalize(gds_path):
    """A canonical, reader-invariant view of a GDS. Geometry and labels are read with
    KLayout (the authoritative geometric reader: every shape is rendered to an
    integer-dbu polygon so path/box representation differences are neutralized).
    Instance transforms are read with gdspy, which exposes the raw GDS STRANS
    rotation / magnification / reflection directly, so a placement divergence is
    reported as the writers actually stored it (KLayout's composed ICplxTrans angle
    can differ from the STRANS field for magnified references). Everything is sorted so
    equal content compares equal regardless of element order."""
    import klayout.db as db
    import gdspy

    ly = db.Layout()
    ly.read(gds_path)
    dbu_um = ly.dbu  # microns per dbu
    layers = [(ly.get_info(li).layer, ly.get_info(li).datatype, li)
              for li in ly.layer_indexes()]

    cells = {}
    census = {"box": 0, "polygon": 0, "path": 0, "text": 0, "sref": 0, "aref": 0}
    for cell in ly.each_cell():
        polys, labels = [], []
        for (lnum, dt, li) in layers:
            for sh in cell.shapes(li).each():
                if sh.is_text():
                    t = sh.text
                    labels.append([lnum, dt, t.trans.disp.x, t.trans.disp.y, t.string])
                    census["text"] += 1
                    continue
                if sh.is_box():
                    census["box"] += 1
                elif sh.is_path():
                    census["path"] += 1
                elif sh.is_polygon() or sh.is_simple_polygon():
                    census["polygon"] += 1
                poly = sh.polygon
                if poly is None:
                    continue
                pts = [[p.x, p.y] for p in poly.each_point_hull()]
                if pts:  # canonicalize the ring: rotate so the min vertex is first
                    k = min(range(len(pts)), key=lambda i: (pts[i][0], pts[i][1]))
                    pts = pts[k:] + pts[:k]
                polys.append([lnum, dt, pts])
        polys.sort()
        labels.sort()
        cells[cell.name] = {"polygons": polys, "labels": labels, "instances": []}

    # Instances (and their sref/aref census) from gdspy's faithful STRANS view.
    glib = gdspy.GdsLibrary()
    glib.read_gds(gds_path)
    to_dbu = lambda um: int(round(um / dbu_um))
    for name, gcell in glib.cells.items():
        if name not in cells:
            cells[name] = {"polygons": [], "labels": [], "instances": []}
        refs = []
        for r in gcell.references:
            is_array = isinstance(r, gdspy.CellArray)
            refs.append([
                r.ref_cell.name if hasattr(r.ref_cell, "name") else str(r.ref_cell),
                round(float(r.rotation or 0.0), 3),
                bool(r.x_reflection),
                to_dbu(float(r.origin[0])), to_dbu(float(r.origin[1])),
                round(float(r.magnification or 1.0), 6),
                bool(is_array),
            ])
            census["aref" if is_array else "sref"] += 1
        refs.sort()
        cells[name]["instances"] = refs
    return {"dbu_nm": round(dbu_um * 1000, 6), "cells": cells, "census": census}


# --------------------------------------------------------------------------- #
# Report
# --------------------------------------------------------------------------- #
TOOLS = ["reticle", "klayout", "gdspy"]


def _diff_cells(ref_name, ref, other_name, other):
    """A list of human-readable divergence lines between two normalized views, each
    line naming which tool produced which value."""
    lines = []
    rc, oc = ref["cells"], other["cells"]
    if set(rc) != set(oc):
        only_ref = sorted(set(rc) - set(oc))
        only_other = sorted(set(oc) - set(rc))
        if only_ref:
            lines.append(f"cells only in {ref_name}: {only_ref}")
        if only_other:
            lines.append(f"cells only in {other_name}: {only_other}")
    for name in sorted(set(rc) & set(oc)):
        a, b = rc[name], oc[name]
        if a["polygons"] != b["polygons"]:
            lines.append(f"cell `{name}`: polygon geometry differs "
                         f"({ref_name}: {len(a['polygons'])} shapes, "
                         f"{other_name}: {len(b['polygons'])} shapes)")
        if a["labels"] != b["labels"]:
            lines.append(f"cell `{name}`: labels differ "
                         f"({ref_name}={a['labels']}, {other_name}={b['labels']})")
        if a["instances"] != b["instances"]:
            lines.append(f"cell `{name}`: instances differ "
                         f"({ref_name}={a['instances']}, {other_name}={b['instances']})")
    return lines


def report(workdir, out_md):
    import os
    fixtures = ["clean", "odd"]
    sections = []
    for fx in fixtures:
        views = {}
        for tool in TOOLS:
            path = f"{workdir}/{fx}.{tool}.gds"
            if os.path.exists(path):
                try:
                    views[tool] = normalize(path)
                except Exception as e:  # noqa: BLE001
                    views[tool] = {"error": str(e)}
        # Also normalize the source fixture for reference.
        src = f"{workdir}/{fx}.gds"
        src_view = normalize(src) if os.path.exists(src) else None

        lines = [f"## Fixture `{fx}.gds`", ""]
        if src_view:
            lines.append(f"Source (gdspy): dbu = {src_view['dbu_nm']} nm, "
                         f"cells = {sorted(src_view['cells'])}, "
                         f"element census = {src_view['census']}.")
            lines.append("")
        # Element-type census per tool (surfaces record-level divergences).
        lines.append("| tool | box | polygon | path | text | sref | aref |")
        lines.append("|------|-----|---------|------|------|------|------|")
        for tool in TOOLS:
            v = views.get(tool)
            if not v:
                lines.append(f"| {tool} | (missing) | | | | | |")
            elif "error" in v:
                lines.append(f"| {tool} | ERROR: {v['error']} | | | | | |")
            else:
                c = v["census"]
                lines.append(f"| {tool} | {c['box']} | {c['polygon']} | {c['path']} "
                             f"| {c['text']} | {c['sref']} | {c['aref']} |")
        lines.append("")
        # Geometry divergences vs the KLayout view (the conformance reference reader).
        ref_tool = "klayout" if "klayout" in views and "error" not in views["klayout"] else None
        if ref_tool:
            for tool in TOOLS:
                if tool == ref_tool or tool not in views or "error" in views[tool]:
                    continue
                diffs = _diff_cells(ref_tool, views[ref_tool], tool, views[tool])
                if diffs:
                    lines.append(f"**{tool} vs {ref_tool}:**")
                    lines.extend(f"- {d}" for d in diffs)
                else:
                    lines.append(f"**{tool} vs {ref_tool}:** geometry, labels, and "
                                 f"instances all match after round-trip.")
                lines.append("")

        # Conformant-OASIS writer acceptance: can KLayout read Reticle's oasis_std output?
        oas = f"{workdir}/{fx}.reticle.oas"
        if os.path.exists(oas):
            import klayout.db as db
            lyo = db.Layout()
            try:
                lyo.read(oas)
                nsh = sum(c.shapes(li).size() for c in lyo.each_cell()
                          for li in lyo.layer_indexes())
                lines.append(f"**OASIS read test:** KLayout read Reticle's conformant-OASIS "
                             f"(`oasis_std`) output - PASS ({lyo.cells()} cells, {nsh} shapes, "
                             f"dbu={lyo.dbu}).")
            except Exception as e:  # noqa: BLE001
                lines.append(f"**OASIS read test:** KLayout FAILED to read Reticle's "
                             f"conformant-OASIS output - {e}")
            lines.append("")
        sections.append("\n".join(lines))

    with open(out_md, "w") as f:
        f.write("\n".join(sections))
    # Also dump the raw normalized JSON alongside, for auditability.
    with open(out_md + ".json", "w") as f:
        allviews = {}
        for fx in fixtures:
            allviews[fx] = {}
            for tool in TOOLS:
                path = f"{workdir}/{fx}.{tool}.gds"
                if os.path.exists(path):
                    try:
                        allviews[fx][tool] = normalize(path)
                    except Exception as e:  # noqa: BLE001
                        allviews[fx][tool] = {"error": str(e)}
        json.dump(allviews, f, indent=2)
    print(f"wrote {out_md}", file=sys.stderr)


# --------------------------------------------------------------------------- #
def oasis_check(oas_path):
    """Try to read a conformant-OASIS file with KLayout and report what it saw. This is
    the acceptance test for Reticle's oasis_std writer: KLayout reading it means the
    file is conformant enough for the reference reader."""
    import klayout.db as db
    ly = db.Layout()
    try:
        ly.read(oas_path)
    except Exception as e:  # noqa: BLE001
        print(f"OASIS-READ FAIL {oas_path}: {e}", file=sys.stderr)
        return 1
    ncells = ly.cells()
    nshapes = sum(cell.shapes(li).size()
                  for cell in ly.each_cell() for li in ly.layer_indexes())
    print(f"OASIS-READ OK {oas_path}: {ncells} cells, {nshapes} shapes, "
          f"dbu={ly.dbu}", file=sys.stderr)
    return 0


def main(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    cmd = argv[1]
    if cmd == "fixtures":
        make_fixtures(argv[2])
    elif cmd == "roundtrip":
        roundtrip(argv[2], argv[3], argv[4])
    elif cmd == "report":
        report(argv[2], argv[3])
    elif cmd == "oasis-check":
        raise SystemExit(oasis_check(argv[2]))
    else:
        raise SystemExit(f"unknown command {cmd!r}\n{__doc__}")


if __name__ == "__main__":
    main(sys.argv)
