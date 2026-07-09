# Information architecture inventory (the Wave 2 contract)

Four contracts in one file: (1) every user-reachable action today, (2) the target
menu tree and panel map, (3) the app.rs region-ownership map for the ten-lane
fan-out, (4) the reserved-CommandId table. Gate 2 verifies the merged registry
against section 4; the shortcut regression test asserts section 1's chords.

## 1. Current actions (harvested from command.rs, keymap.rs, and the panel code)

Chords are the shipped defaults (`keymap.rs` `defaults()`); freq is an estimate
(H daily, M weekly, L rare) used only to pick menu placement and collapse flags.

| action | current home | chord | freq |
|---|---|---|---|
| Tool: Select / Pan / Measure | toolbar selectables + palette | V / S / M | H |
| Tool: Cut line / Draw rect / Draw polygon / Draw path / Edit vertices | toolbar + palette | none | H |
| Path width + endcap options | toolbar (inline when Draw path) | none | M |
| Open (back to start screen) | toolbar button | none | H |
| Convert GDS to archive (web) | toolbar button | none | M |
| Fit design | toolbar + palette | F | H |
| Undo / Redo | toolbar + History + palette | Ctrl+Z / Ctrl+Y | H |
| Toggle Grid / Snap / Labels / Minimap | toolbar checkboxes | Ctrl+G / none / L / N | M |
| Split Single / H / V | toolbar selectables | Ctrl+1 / Ctrl+2 / Ctrl+3 | M |
| Command palette | toolbar button | Ctrl+P | H |
| Keyboard shortcuts (rebind window) | toolbar button | none | L |
| Take the tour / Core tour only | Help menu (the only menu) | none | L |
| Switch replay theater / editor (web) | toolbar button | none | M |
| Layer visibility per row; Show all / Hide all; filter | Layers panel | none | H |
| Toggle layer N / Select all on layer N | palette (dynamic) | none | M |
| Select by layer name | Layers panel | none | M |
| Add demo rectangle | History section button | none | L (debug) |
| Step back / Step fwd (undo steps) | History section | none | M |
| Run DRC / Clear; Check as you type; Ask agent to fix | DRC section | none | H |
| Snapshot / Diff vs snapshot / Clear; Show diff overlay | Diff section | none | M |
| Add comment; resolve/select pins | Comments section | none | M |
| Agent Run / Stop; Replay theater; Replay this run | Agent section | none | M |
| Share this session; copy link / viewer link / permalink | Share section | none | M |
| Follow the sharer's view (viewer) | Share section checkbox | none | H (viewer) |
| Boolean ops (union etc.) | Operations section | none | M |
| Cut / Paste / Duplicate / Move / Array / via stack | Productivity section | none | M |
| Generate (catalog + form + preview) | Generate section | none | M |
| Snap to geometry / guides; guide add H / V / clear | Snap section | none | M |
| Toggle theme; camera bookmarks (save/recall/x); Export SVG/PNG scope | View and export section | none | L |
| Search filter; saved selection sets; select matching; outline tree | Search section | none | M |
| Layer reorder / recolor / fill / solo; tech round-trip | Technology editor section | none | L |
| Replay theater transport (load, step, play, speed) | theater window | none | M |
| Rebind / Clear / Reset shortcuts; Save (native) | shortcuts window | none | L |

**Shortcut regression list (the 1E unit test asserts exactly these defaults):**
Ctrl+P palette, Ctrl+Z undo, Ctrl+Y redo, F fit, Ctrl+G grid, V select, S pan,
M measure, L labels, N minimap, Ctrl+1/2/3 splits; `toggle_snap` ships unbound.
Saved `keymap.toml` files with the 14 legacy tags keep loading (alias table).

## 2. Target structure

### Menu tree (rendered from the registry; hints show live chords)

- **File**: Open... (Ctrl+O) | Open from URL... | Open Recent > (list, clear) |
  Convert GDS to archive... (web) | Export > View as PNG / View as SVG /
  Metrology CSV | Close design (back to Start)
- **Edit**: Undo (Ctrl+Z) | Redo (Ctrl+Y) | Duplicate (Ctrl+D) | Cut / Paste |
  Move... | Array... | Via stack... | Boolean > Union / Intersect / Subtract
- **View**: Fit (F) | Fit selection (Shift+F) | Zoom 1:1 DBU | Zoom to layer
  extents | Bookmarks > save/recall | Grid (Ctrl+G) | Snap | Labels (L) |
  Minimap (N) | Rulers | Split > Single (Ctrl+1) / Horizontal (Ctrl+2) /
  Vertical (Ctrl+3) | Panels > 3D stack / Cross-section / Toggle panels (Tab) |
  Presentation mode (P)
- **Select**: Clear | All on layer... | Same layer as selection | By name... |
  Saved selection sets >
- **Draw**: Select (V) | Pan (S) | Measure (M) | Cut line | Rectangle | Polygon |
  Path | Edit vertices | Path options...
- **Verify**: Run DRC | Check as you type | Clear DRC | Snapshot for diff |
  Diff vs snapshot | Show diff overlay | Clear diff
- **Share**: Share this session... | Copy permalink at this view | Copy viewer
  link | Comments >
- **Help**: Take the tour | Shortcuts (?) | Documentation | What's new |
  Settings... | About Reticle | Developer > Add demo rectangle / Replay theater

Every menu item is a registry entry; the palette shows all of them plus
palette-only targets (layers, cells, bookmarks, goto). Nothing is menu-only or
palette-only in the registry data (`menu_path` or palette visibility, checked by
the 1E parity test).

### Panel map

- **Left, Layers** (2C): swatch, visibility eye, hover peek, alt-click solo,
  filter with clear, show/hide-all icons. In archive/viewer modes it reflects
  the ACTIVE document, never the built-in demo (AUD-04).
- **Right, Inspector** (2B): segmented control over four groups; sections are
  1C collapsible components with persisted open flags and empty states.
  - Inspect: Properties (selection details), Search and outline, History.
  - Review: DRC, Layout diff, Comments.
  - Automate: Agent (prompt, run state, transcript, replay), Generate.
  - Settings: Operations, Productivity, Snap and guides, Export, Technology
    editor. (Theme toggle is retired per tokens.md; density/motion move to the
    Settings dialog, 4C; camera bookmarks move to the View menu, 3A.)
- **Bottom, status bar** (3A): tool | coordinates (mono, tabular) | selection
  summary | zoom (click: presets) | fps (click: perf popover) | archive label
  (click: HUD) | transient status.
- **Overlays** (3A layout manager): streaming HUD top-left below the rulers,
  legends bottom-right, coordinate readout in the status bar only; overlays
  never intersect rulers, panels, or each other by construction.
- **3D stack / Cross-section** (2C): managed panels per ADR 0096, opened from
  View > Panels, never floating.

### Viewer chrome (2D)

A share link opens: canvas, status bar, Layers panel, presence cursors, a session
chip (connection state, viewer count), a follow toggle, and one fixed
"Open full editor" affordance top-right. No Inspector, no draw tools, no menu bar
except Help. The viewer banner states view-only clearly (catalog 23). The full
editor is one click away and the transition preserves the camera.

## 3. app.rs region-ownership map (Wave 2, ten lanes)

A lane edits only its regions plus its own new files; shared surfaces are
append-only marked sections. Function names as of `ec9a851`.

| lane | owns (app.rs regions) | owns (files) |
|---|---|---|
| 2A | `toolbar`, `help_menu`, new `menubar`, `add_demo_rectangle` relocation | (new) menu module |
| 2B | `main_panels` right-panel block, `history_panel`, `inspector_panel`, `drc_panel`, `diff_panel`, `comment_panel`, `agent_section`+plan/conversation/history, `ops_panel` hosting, `productivity_panel`, `generate_section`, `snap_panel`, `view_export_panel`, `search_panel`, `tech_editor_panel` hosting | inspector layout module (new) |
| 2C | `layer_panel`, `main_panels` left-panel block, `draw_minimap` placement hook | `view3d.rs`, `xsection.rs`, `layers.rs`, `minimap.rs` |
| 2D | `start_screen_ui` + start sections, `share_section`, `viewer_share_controls`, viewer-mode gating in `ui`/`main_panels` | `startscreen.rs`, `usecases.rs`, `viewer.rs` chrome |
| 3A | `canvas`, `draw_rulers`, `draw_guides`, `draw_snap_indicator`, `draw_archive`+HUD, `draw_minimap` internals, `status_bar`, zoom/fit/animation paths, splash hook in `crates/web` | `streamed.rs`, `archive.rs`, overlay manager (new), `fps.rs` |
| 3B | open flow (`open_*`, progress/error windows), toast adoption, dialogs | `webopen.rs`, `notify.rs`, dialog modules (new) |
| 3C | `palette_window`, `keymap_window`, `handle_shortcuts`, `run_action`/`dispatch` glue, context menus, focus traversal | `commands.rs`, `command.rs`, `keymap.rs` |
| 4A | component states and transitions only | `theme/components.rs` (additive-only; public signatures frozen) |
| 4B | density/touch values, text-scaling checks | touch e2e recipe, `theme/tokens.rs` touch constants |
| 4C | tour overlay hook, Help content, Settings dialog, hints | `tour.rs`, settings module (new), `session.rs` settings keys |

**Shared append-only surfaces** (marked per-lane sections, appended at the end of
the marked block; the orchestrator resolves residual overlaps at merge):
`crates/reticle-app/src/commands.rs` registry table, `session.rs` keys +
`SessionState` fields, `docs/src/SUMMARY.md`, `justfile` recipes,
`crates/reticle-app/src/theme/icons.rs` glyph constants (1B's generator ordering
is canonical; lanes request glyphs via brief notes if missing).

**Also frozen for everyone**: the e2e seams (`__reticle_stats` keys are
add-only; query params `view/archive/gds/share/room/relay/e2e-edit/cell/layers`
keep their exact semantics; console signals `reticle-live: socket open` and
`first frame` stay), `run_command`'s existing arms, and the wasm boot contract
(`#overlay` hides only after WebRunner start).

Packet v8.1.0-R adds, additively (existing keys untouched), the standing
demo-observability seam keys and one e2e-only query param:
`__reticle_stats.hash_check` (the replay verdict `Match`/`Mismatch`/`Pending`/
`Unverifiable`, previously canvas-only), `__reticle_stats.render_nonblank` (a
bool: the app is painting real geometry with a live camera this frame),
`__reticle_stats.applied_scene_shapes` (the flattened renderable shape count,
which counts a hierarchical design's instance-expanded geometry, unlike the
top-cell-direct `applied_shapes`), and `__reticle_stats.applied_shapes` now also
published on every editor frame (it was viewer-path only). Two e2e-only params:
`?e2e-autoplay=1` starts the replay theater playing on boot so a headed guard can
read `hash_check` without clicking the GPU-painted transport (the public
`?view=replay` landing still waits at Play), and `?e2e-example=<id>` (`tt03` /
`sky130`) boots straight into a compiled-in example, since the Start-screen cards
are canvas-painted and not DOM-clickable. These are add-only: the pre-existing
keys and param semantics are unchanged, so the seam canaries keep passing.

## 4. Reserved CommandIds (cross-lane wiring contract)

Gate 2 asserts every id below exists in the merged registry with this exact
spelling, menu path, and default chord (none = unbound). Owner = the lane that
implements the effect; any lane may reference the id (menus, palette, context
menus, buttons) without coordination. Ids a lane invents for purely internal
commands must use its prefixes and appear in its marked registry section.

| id | label | owner | menu path | chord |
|---|---|---|---|---|
| file.open_dialog | Open... | 3B | File | Ctrl+O |
| file.open_url | Open from URL... | 3B | File | none |
| file.convert_gds | Convert GDS to archive... | 3B | File | none |
| file.close_design | Close design | 2D | File | none |
| file.export_png | Export view as PNG | 2B | File > Export | none |
| file.export_svg | Export view as SVG | 2B | File > Export | none |
| file.export_metrology | Export metrology CSV | 2B | File > Export | none |
| edit.undo | Undo | 1E | Edit | Ctrl+Z |
| edit.redo | Redo | 1E | Edit | Ctrl+Y |
| edit.duplicate | Duplicate | 3A | Edit | Ctrl+D |
| edit.cut | Cut | 2B | Edit | none |
| edit.paste | Paste | 2B | Edit | none |
| edit.move_exact | Move... | 2B | Edit | none |
| edit.array | Array... | 2B | Edit | none |
| edit.via_stack | Via stack... | 2B | Edit | none |
| edit.bool_union | Boolean union | 2B | Edit > Boolean | none |
| edit.bool_intersect | Boolean intersect | 2B | Edit > Boolean | none |
| edit.bool_subtract | Boolean subtract | 2B | Edit > Boolean | none |
| view.zoom_fit | Fit | 1E | View | F |
| view.zoom_selection | Fit selection | 3A | View | Shift+F |
| view.zoom_one_to_one | Zoom 1:1 DBU | 3A | View | none |
| view.zoom_layer_extents | Zoom to layer extents | 3A | View | none |
| view.bookmark_save | Save view bookmark | 3A | View > Bookmarks | none |
| view.grid | Grid | 1E | View | Ctrl+G |
| view.snap | Snap | 1E | View | none |
| view.labels | Labels | 1E | View | L |
| view.minimap | Minimap | 1E | View | N |
| view.rulers | Rulers | 3A | View | none |
| view.split_single | Single pane | 1E | View > Split | Ctrl+1 |
| view.split_h | Split horizontal | 1E | View > Split | Ctrl+2 |
| view.split_v | Split vertical | 1E | View > Split | Ctrl+3 |
| view.panel_3d | 3D stack panel | 2C | View > Panels | none |
| view.panel_xsection | Cross-section panel | 2C | View > Panels | none |
| view.panels_toggle | Toggle panels | 2B | View > Panels | Tab |
| view.presentation | Presentation mode | 2D | View | P |
| select.clear | Clear selection | 1E | Select | none |
| select.same_layer | Same layer as selection | 2B | Select | none |
| select.by_name | Select by name... | 2B | Select | none |
| tool.select | Select tool | 1E | Draw | V |
| tool.pan | Pan tool | 1E | Draw | S |
| tool.measure | Measure tool | 1E | Draw | M |
| tool.cutline | Cut line tool | 1E | Draw | none |
| tool.rect | Rectangle tool | 1E | Draw | none |
| tool.polygon | Polygon tool | 1E | Draw | none |
| tool.path | Path tool | 1E | Draw | none |
| tool.vertices | Edit vertices tool | 1E | Draw | none |
| verify.drc_run | Run DRC | 2B | Verify | none |
| verify.drc_live | Check as you type | 2B | Verify | none |
| verify.drc_clear | Clear DRC results | 2B | Verify | none |
| verify.diff_snapshot | Snapshot for diff | 2B | Verify | none |
| verify.diff_run | Diff vs snapshot | 2B | Verify | none |
| verify.diff_overlay | Show diff overlay | 2B | Verify | none |
| share.dialog | Share this session... | 3B | Share | none |
| share.copy_permalink | Copy permalink at this view | 3A | Share | none |
| share.copy_viewer_link | Copy viewer link | 3B | Share | none |
| comment.add | Add comment | 2B | Share > Comments | none |
| palette.open | Command palette | 1E | Help | Ctrl+P |
| palette.goto_coordinate | Go to coordinate... | 3C | (palette only) | none |
| palette.goto_cell | Go to cell... | 3C | (palette only) | none |
| focus.cycle | Cycle focus region | 3C | (palette only) | F6 |
| help.tour | Take the tour | 4C | Help | none |
| help.shortcuts | Keyboard shortcuts | 3C | Help | ? |
| help.docs | Documentation | 4C | Help | none |
| help.whats_new | What's new | 4C | Help | none |
| help.settings | Settings... | 4C | Help | none |
| help.about | About Reticle | 4C | Help | none |
| dev.add_demo_rect | Insert demo rectangle | 2A | Help > Developer | none |
| dev.replay_theater | Replay theater | 2A | Help > Developer | none |

Chord-conflict note: new defaults (Ctrl+O, Ctrl+D, Shift+F, Tab, P, ?, F6) do not
collide with the regression list; `Tab`/`P`/`?` are suppressed while a text field
has focus (existing `handle_shortcuts` rule); Esc is the cascade (catalog 84) and
is not a bindable command. `view.presentation` (P) is owned by 2D with 4C
providing the Help copy.

## 5. New query params introduced this packet (additive; parsers stay pure)

| param | meaning | owner |
|---|---|---|
| `?gallery=1` | component gallery screen (screenshot surface) | 1C |
| `?tour=1` | boot straight into the guided tour | 4C |
| `?embed=1` | minimal chrome for iframes (P2; ledger if squeezed) | 2D |
